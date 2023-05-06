use super::{
    ExchangeClient, ExchangeClientError, ExchangeIdentifier, SummaryReceiver, SummarySender,
};
use common::CurrencyPair;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use orderbook as ob;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    net::TcpStream,
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        oneshot::{channel as oneshot_channel, Sender as OneshotSender},
        Mutex,
    },
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, Result},
    MaybeTlsStream, WebSocketStream,
};
use url::Url;

#[derive(Debug)]
pub struct Client {
    sub_tx: UnboundedSender<(
        CurrencyPair,
        OneshotSender<Result<SummaryReceiver, ExchangeClientError>>,
    )>,
}

impl Client {
    pub async fn connect() -> Result<Self, ExchangeClientError> {
        let (socket, _) = connect_async(Url::parse("wss://ws.bitstamp.net").expect("Invalid URL"))
            .await
            .map_err(|err| ExchangeClientError::Connect(Box::new(err)))?;

        let (socket_tx, socket_rx) = socket.split();

        let stream_rx_map = Arc::new(Mutex::new(HashMap::<CurrencyPair, SummarySender>::new()));

        let (sub_tx, sub_rx) = unbounded_channel();

        tokio::spawn(async move {
            ClientInner {
                sub_rx,
                socket_tx,
                socket_rx,
                stream_rx_map,
            }
            .run()
            .await;
        });

        Ok(Self { sub_tx })
    }
}

#[async_trait::async_trait]
impl ExchangeClient for Client {
    async fn get_summary_stream(
        &mut self,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError> {
        let (resp_tx, resp_rx) = oneshot_channel();

        self.sub_tx
            .send((currency_pair, resp_tx))
            .expect("ClientInner died");

        resp_rx.await.expect("ClientInner died")
    }
}

struct ClientInner {
    sub_rx: UnboundedReceiver<(
        CurrencyPair,
        OneshotSender<Result<SummaryReceiver, ExchangeClientError>>,
    )>,
    socket_tx: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    socket_rx: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    stream_rx_map: Arc<Mutex<HashMap<CurrencyPair, SummarySender>>>,
}

impl ClientInner {
    async fn run(&mut self) {
        loop {
            tokio::select! {
                x = self.sub_rx.recv() => {
                    let Some((currency_pair, resp_tx)) = x else {
                        continue;
                    };

                    // If this errors, it's ok - when the first message comes in from the exchange
                    // and we see the other channel was dropped, we'll unsubscribe.
                    resp_tx.send(self.handle_subscribe(currency_pair).await).ok();
                }
                opt = self.socket_rx.next() => {
                    if let Some(res) = opt {
                        match res {
                            Ok(msg) => {
                                if self.handle_message(msg).await {
                                    break;
                                }
                            },
                            Err(err) => {
                                tracing::error!("Bitstamp socket error: {}", err);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn handle_subscribe(
        &mut self,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError> {
        let event = Event::new_subscribe(currency_pair);

        self.socket_tx
            .send(Message::Text(serde_json::to_string(&event).unwrap()))
            .await
            .expect("Could not send");

        let (tx, rx) = unbounded_channel();

        self.stream_rx_map.lock().await.insert(currency_pair, tx);

        Ok(rx)
    }

    async fn handle_message(&mut self, msg: Message) -> bool {
        match msg {
            Message::Text(text) => match serde_json::from_str::<WsResponse>(&text) {
                Ok(resp) => self.handle_ws_resp(resp).await,
                Err(_) => {
                    // This can happen when we receive a "subscription successful" message.
                    // Let's just ignore these for now.
                }
            },
            Message::Close(_) => return true,
            _ => {}
        }

        false // Don't close
    }

    async fn handle_ws_resp(&mut self, resp: WsResponse) {
        let currency_pair = resp.get_currency_pair();

        let mut locked_map = self.stream_rx_map.lock().await;

        match locked_map.get(&currency_pair) {
            Some(tx) => {
                let summary = resp_to_summary(resp);

                if tx
                    .send((ExchangeIdentifier::Bitstamp, currency_pair, summary))
                    .is_err()
                {
                    // Shouldn't happen
                    locked_map.remove(&currency_pair);
                }
            }
            None => {
                // The subscriber has dropped the channel; we can unsubscribe
                let event = Event::new_unsubscribe(currency_pair);

                self.socket_tx
                    .send(Message::Text(serde_json::to_string(&event).unwrap()))
                    .await
                    .expect("Could not send");
            }
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct Event {
    event: String,
    data: EventData,
}

impl Event {
    fn new_subscribe(currency_pair: CurrencyPair) -> Self {
        Self {
            event: String::from("bts:subscribe"),
            data: EventData {
                channel: format!("order_book_{}", currency_pair_to_str(currency_pair)),
            },
        }
    }

    fn new_unsubscribe(currency_pair: CurrencyPair) -> Self {
        Self {
            event: String::from("bts:unsubscribe"),
            data: EventData {
                channel: format!("order_book_{}", currency_pair_to_str(currency_pair)),
            },
        }
    }
}

fn currency_pair_to_str(currency_pair: CurrencyPair) -> &'static str {
    match currency_pair {
        CurrencyPair::BtcGbp => "btcgbp",
        CurrencyPair::BtcUsd => "btcusd",
        CurrencyPair::EthBtc => "ethbtc",
    }
}

#[derive(Debug, serde::Serialize)]
struct EventData {
    channel: String,
}

#[derive(Debug, serde::Deserialize)]
struct WsResponse {
    // event: String,
    channel: String,
    data: WsResponseData,
}

impl WsResponse {
    fn get_currency_pair(&self) -> CurrencyPair {
        // Strip "order_book_" from the front
        match &self.channel[11..] {
            "btcgbp" => CurrencyPair::BtcGbp,
            "btcusd" => CurrencyPair::BtcUsd,
            "ethbtc" => CurrencyPair::EthBtc,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct WsResponseData {
    bids: Vec<(String, String)>,
    asks: Vec<(String, String)>,
}

fn resp_to_summary(pbd: WsResponse) -> Result<ob::Summary, ExchangeClientError> {
    let bids: Result<Vec<_>, _> = pbd
        .data
        .bids
        .into_iter()
        .map(|(price_str, amount_str)| string_pair_to_level(&price_str, &amount_str))
        .collect();

    let asks: Result<Vec<_>, _> = pbd
        .data
        .asks
        .into_iter()
        .map(|(price_str, amount_str)| string_pair_to_level(&price_str, &amount_str))
        .collect();

    Ok(ob::Summary {
        spread: 0.0,
        bids: bids?,
        asks: asks?,
    })
}

fn string_pair_to_level(
    price_str: &str,
    amount_str: &str,
) -> Result<ob::Level, ExchangeClientError> {
    Ok(ob::Level {
        exchange: String::from("bitstamp"),
        price: price_str
            .parse()
            .map_err(|_| ExchangeClientError::InvalidDecimal)?,
        amount: amount_str
            .parse()
            .map_err(|_| ExchangeClientError::InvalidDecimal)?,
    })
}
