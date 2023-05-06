use super::exchanges::{
    BinanceClient, BitstampClient, ExchangeClient, ExchangeClientError,
    SummaryReceiver as ExchangeSummaryReceiver,
};
use common::{CurrencyPair, ExchangeIdentifier};
use futures::stream::FuturesUnordered;
use fxhash::{FxHashMap, FxHashSet};
use orderbook as ob;
use std::collections::hash_map::Entry;
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot::{channel as oneshot_channel, Sender as OneshotSender},
};
use tokio_stream::StreamExt;

pub type SummaryReceiver = UnboundedReceiver<ob::Summary>;

#[derive(Debug, Clone)]
pub struct Aggregator {
    sub_tx: UnboundedSender<Subscribe>,
}

impl Aggregator {
    pub fn new() -> Self {
        let (inner_tx, inner_rx) = unbounded_channel();

        tokio::spawn(async move {
            AggregatorInner::new(inner_rx).run().await;
        });

        Self { sub_tx: inner_tx }
    }

    pub async fn get_summary_stream(
        &self,
        exchange_idents: Vec<ExchangeIdentifier>,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError> {
        let (resp_tx, resp_rx) = oneshot_channel();

        self.sub_tx
            .send(Subscribe {
                exchange_idents,
                currency_pair,
                resp_tx,
            })
            .expect("AggregatorInner died");

        resp_rx.await.expect("AggregatorInner died")
    }
}

struct AggregatorInner {
    sub_rx: UnboundedReceiver<Subscribe>,
    // Because the number of exchanges will be small, this would be faster as a `Vec`
    client_map: FxHashMap<ExchangeIdentifier, Box<dyn ExchangeClient>>,
    pair_map: FxHashMap<(ExchangeIdentifier, CurrencyPair), PairData>,
    subscriber_map: FxHashMap<usize, SubscriberData>,
    next_subscriber_index: usize,
}

impl AggregatorInner {
    fn new(sub_rx: UnboundedReceiver<Subscribe>) -> Self {
        Self {
            sub_rx,
            client_map: FxHashMap::default(),
            pair_map: FxHashMap::default(),
            subscriber_map: FxHashMap::default(),
            next_subscriber_index: 0,
        }
    }

    async fn run(&mut self) {
        loop {
            // `FuturesUnordered::next()` immediately returns `None` if there are no futures inside.
            // To avoid this, we add a channel that never returns.
            let (_never_tx, mut never_rx) = unbounded_channel();

            // Aggregate
            let mut update_rx = self
                .pair_map
                .values_mut()
                .map(|pair_data| pair_data.summary_rx.recv())
                .chain(std::iter::once(never_rx.recv()))
                .collect::<FuturesUnordered<_>>();

            tokio::select! {
                opt = update_rx.next() => {
                    drop(update_rx);

                    match opt {
                        Some(Some((exchange_ident, currency_pair, res))) => {
                            self.handle_update(exchange_ident, currency_pair, res).await;
                        },
                        Some(None) => {
                            // One of the channels has closed. We can't be sure which because `FuturesUnordered`
                            // doesn't tell us the index of the future that yielded first.
                            //tracing::warn!("AggregatorInner: an update_rx channel returned None");
                        }
                        None => {
                            unreachable!("update_rx always contains one or more futures")
                        }
                    }
                }
                opt = self.sub_rx.recv() => {
                    drop(update_rx);

                    match opt {
                        Some(subscribe) => {
                            let res = self.handle_subscribe(subscribe.exchange_idents, subscribe.currency_pair).await;
                            if subscribe.resp_tx.send(res).is_err() {
                                // TODO: Unsubscribe
                            }
                        },
                        None => return
                    }
                }
            };
        }
    }

    async fn handle_subscribe(
        &mut self,
        exchange_idents: Vec<ExchangeIdentifier>,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError> {
        let _added_exchange_idents = self.ensure_clients(&exchange_idents).await?;

        let mut subscribed_exchange_idents = vec![];
        let mut subscribe_err = None;

        for exchange_ident in exchange_idents.iter() {
            match self
                .ensure_summary_stream(*exchange_ident, currency_pair)
                .await
            {
                Ok(_) => {
                    subscribed_exchange_idents.push(*exchange_ident);
                }
                Err(err) => {
                    subscribe_err = Some(err);
                    break;
                }
            }
        }

        if let Some(err) = subscribe_err {
            // TODO:
            // - Unsubscribe to all `subscribed_exchange_idents`
            // - Disconnect all `added_exchange_idents`
            return Err(err);
        }

        // Record the subscriber for each ensured pair
        for exchange_ident in exchange_idents.iter() {
            self.pair_map
                .get_mut(&(*exchange_ident, currency_pair))
                .unwrap()
                .subscriber_indices
                .insert(self.next_subscriber_index);
        }

        let (summary_tx, summary_rx) = unbounded_channel();

        self.subscriber_map.insert(
            self.next_subscriber_index,
            SubscriberData {
                exchange_idents,
                currency_pair,
                summary_tx,
            },
        );

        // Do an immediate notify just in case an update doesn't come through for a while
        self.notify_subscriber(self.next_subscriber_index).await;

        // Update index
        self.next_subscriber_index += 1;

        Ok(summary_rx)
    }

    async fn handle_update(
        &mut self,
        exchange_ident: ExchangeIdentifier,
        currency_pair: CurrencyPair,
        res: Result<ob::Summary, ExchangeClientError>,
    ) {
        let summary = match res {
            Ok(summary) => summary,
            Err(err) => {
                // There was a problem with the exchange client. Our interface doesn't
                // currently allow these problems to be forwarded downstream. We should
                // probably look at the type of problem here and handle it (reconnecting
                // to the exchange, killing the subscription, etc). For now, just print
                // and ignore.
                println!(
                    "AggregatorInner::handle_update got error from {} for {}: {}",
                    exchange_ident, currency_pair, err
                );
                return;
            }
        };

        let Some(pair_data) = self.pair_map.get_mut(&(exchange_ident, currency_pair)) else {
            // We don't have a reference to this combination anymore. This could (maybe?)
            // happen if there's an unsubscribe and we have a result waiting to come in.
            return;
        };

        // Update the last_order_book
        pair_data.last_orderbook.last_bids = summary.bids;
        pair_data.last_orderbook.last_asks = summary.asks;

        // Now update subscribers. It's a bit annoying that we have to clone
        // `subscriber_indices` here, but if we don't, the compiler complains
        // about mutable and immutable references to `pair_data`. This is the
        // easiest solution...
        let subscriber_indices = pair_data
            .subscriber_indices
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        tracing::info!("Got {} update from {}", currency_pair, exchange_ident);

        // Could do a `tokio::join_all` here
        let mut indices_to_delete = vec![];

        for subscriber_index in subscriber_indices {
            if self.notify_subscriber(subscriber_index).await {
                indices_to_delete.push(subscriber_index);
            }
        }

        indices_to_delete.into_iter().for_each(|subscriber_index| {
            self.unsubscribe(subscriber_index);
        })
    }

    fn unsubscribe(&mut self, subscriber_index: usize) {
        let Some(subscriber_data) = self.subscriber_map.remove(&subscriber_index) else {
            return;
        };

        let mut pair_keys_to_delete = vec![];

        for exchange_ident in subscriber_data.exchange_idents {
            let pair_key = (exchange_ident, subscriber_data.currency_pair);

            let Some(pair_data) = self.pair_map.get_mut(&pair_key) else {
                continue;
            };

            pair_data.subscriber_indices.remove(&subscriber_index);

            if pair_data.subscriber_indices.is_empty() {
                tracing::info!("Last subscriber for pair");
                pair_keys_to_delete.push(pair_key);
            }
        }

        for pair_key in pair_keys_to_delete {
            self.pair_map.remove(&pair_key);
        }
    }

    /// Returns `true` if the subscriber has dropped and needs to be removed
    async fn notify_subscriber(&self, subscriber_index: usize) -> bool {
        let Some(subscriber_data) = self.subscriber_map.get(&subscriber_index) else {
            // Shouldn't happen
            return false;
        };

        let mut all_bids = vec![];
        let mut all_asks = vec![];

        // Aggregate all relevant order books
        for exchange_ident in subscriber_data.exchange_idents.iter() {
            let pair_data = self
                .pair_map
                .get(&(*exchange_ident, subscriber_data.currency_pair))
                .unwrap();

            all_bids.extend(pair_data.last_orderbook.last_bids.iter());
            all_asks.extend(pair_data.last_orderbook.last_asks.iter());
        }

        // Sort and truncate
        all_bids.sort_by(|l1, l2| l2.price.partial_cmp(&l1.price).unwrap());
        all_bids.truncate(10);

        all_asks.sort_by(|l1, l2| l1.price.partial_cmp(&l2.price).unwrap());
        all_asks.truncate(10);

        // Now convert to proto object
        let spread = match (all_bids.first(), all_asks.first()) {
            (Some(bid), Some(ask)) => ask.price - bid.price,
            _ => 0.0,
        };

        // Defer the clone until we have our final (length <= 10) vecs
        let summary = ob::Summary {
            spread,
            bids: all_bids.into_iter().cloned().collect(),
            asks: all_asks.into_iter().cloned().collect(),
        };

        subscriber_data.summary_tx.send(summary).is_err()
    }

    /// Makes sure that all exchanges specified have been connected to.
    /// If any exchange could not be connected to, no exchanges will be added.
    /// On success, returns all new exchange identifiers added.
    async fn ensure_clients(
        &mut self,
        exchange_idents: &[ExchangeIdentifier],
    ) -> Result<Vec<ExchangeIdentifier>, ExchangeClientError> {
        let mut added = vec![];
        let mut any_err = None;

        for exchange_ident in exchange_idents {
            match self.client_map.entry(*exchange_ident) {
                Entry::Occupied(_) => {}
                Entry::Vacant(entry) => match make_exchange_client(*exchange_ident).await {
                    Ok(client) => {
                        entry.insert(client);
                        added.push(*exchange_ident);
                    }
                    Err(err) => {
                        any_err = Some(err);
                        break;
                    }
                },
            }
        }

        match any_err {
            Some(err) => {
                added.iter().for_each(|exchange_ident| {
                    self.client_map.remove(exchange_ident);
                });
                Err(err)
            }
            None => Ok(added),
        }
    }

    async fn ensure_summary_stream(
        &mut self,
        exchange_ident: ExchangeIdentifier,
        currency_pair: CurrencyPair,
    ) -> Result<(), ExchangeClientError> {
        if let Entry::Vacant(entry) = self.pair_map.entry((exchange_ident, currency_pair)) {
            // We haven't subscribed to this stream yet. We know the client is ensured
            // so it's safe to `unwrap` the client here.
            let summary_rx = self
                .client_map
                .get_mut(&exchange_ident)
                .unwrap()
                .get_summary_stream(currency_pair)
                .await?;

            entry.insert(PairData {
                summary_rx,
                last_orderbook: LastOrderbook::default(),
                subscriber_indices: FxHashSet::default(),
            });
        }

        Ok(())
    }
}

async fn make_exchange_client(
    exchange_ident: ExchangeIdentifier,
) -> Result<Box<dyn ExchangeClient>, ExchangeClientError> {
    Ok(match exchange_ident {
        ExchangeIdentifier::Binance => Box::new(BinanceClient),
        ExchangeIdentifier::Bitstamp => Box::new(BitstampClient::connect().await?),
    })
}

#[derive(Debug)]
struct Subscribe {
    exchange_idents: Vec<ExchangeIdentifier>,
    currency_pair: CurrencyPair,
    resp_tx: OneshotSender<Result<SummaryReceiver, ExchangeClientError>>,
}

struct PairData {
    summary_rx: ExchangeSummaryReceiver,
    last_orderbook: LastOrderbook,
    subscriber_indices: FxHashSet<usize>,
}

#[derive(Default)]
struct LastOrderbook {
    last_bids: Vec<ob::Level>,
    last_asks: Vec<ob::Level>,
}

struct SubscriberData {
    exchange_idents: Vec<ExchangeIdentifier>,
    currency_pair: CurrencyPair,
    summary_tx: UnboundedSender<ob::Summary>,
}
