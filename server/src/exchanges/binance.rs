use super::{ExchangeClient, ExchangeClientError, ExchangeIdentifier, SummaryReceiver};
use common::CurrencyPair;
use futures_util::StreamExt;
use orderbook as ob;
use tokio::sync::mpsc::unbounded_channel;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, Result},
};
use url::Url;

#[derive(Debug)]
pub struct Client;

fn make_url(currency_pair: CurrencyPair) -> Url {
    let currency_pair_str = match currency_pair {
        CurrencyPair::BtcGbp => "btcgbp",
        CurrencyPair::BtcUsd => "btcusd",
        CurrencyPair::EthBtc => "ethbtc",
    };

    let url_str = format!("wss://stream.binance.com:9443/ws/{currency_pair_str}@depth10@100ms");

    Url::parse(&url_str).expect("Invalid URL")
}

#[async_trait::async_trait]
impl ExchangeClient for Client {
    async fn get_summary_stream(
        &mut self,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError> {
        let (mut socket, _) = connect_async(make_url(currency_pair))
            .await
            .map_err(|err| ExchangeClientError::Connect(Box::new(err)))?;

        let (tx, rx) = unbounded_channel();

        tokio::spawn(async move {
            while let Some(res) = socket.next().await {
                match res {
                    Ok(msg) => match msg {
                        Message::Text(text) => {
                            match serde_json::from_str::<WsResponse>(&text) {
                                Ok(pbd) => {
                                    if tx
                                        .send((
                                            ExchangeIdentifier::Binance,
                                            currency_pair,
                                            resp_to_summary(pbd),
                                        ))
                                        .is_err()
                                    {
                                        // Other side has dropped the channel; we can return
                                        // (and hence drop the socket).
                                        break;
                                    }
                                }
                                Err(err) => println!("Error deserialising book depth: {}", err),
                            }
                        }
                        Message::Close(_) => {
                            break;
                        }
                        _ => {}
                    },
                    Err(err) => {
                        tracing::error!("Binance socket error: {}", err);
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsResponse {
    // last_update_id: usize,
    bids: Vec<(String, String)>,
    asks: Vec<(String, String)>,
}

fn resp_to_summary(pbd: WsResponse) -> Result<ob::Summary, ExchangeClientError> {
    let bids: Result<Vec<_>, _> = pbd
        .bids
        .into_iter()
        .map(|(price_str, amount_str)| string_pair_to_level(&price_str, &amount_str))
        .collect();

    let asks: Result<Vec<_>, _> = pbd
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
        exchange: String::from("binance"),
        price: price_str
            .parse()
            .map_err(|_| ExchangeClientError::InvalidDecimal)?,
        amount: amount_str
            .parse()
            .map_err(|_| ExchangeClientError::InvalidDecimal)?,
    })
}
