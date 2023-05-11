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
    pub fn new(ecf: Box<dyn ExchangeClientFactory>) -> Self {
        let (inner_tx, inner_rx) = unbounded_channel();

        tokio::spawn(async move {
            AggregatorInner::new(ecf, inner_rx).run().await;
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

impl Default for Aggregator {
    fn default() -> Self {
        Self::new(Box::new(RealExchangeClientFactory))
    }
}

struct AggregatorInner {
    ecf: Box<dyn ExchangeClientFactory>,
    sub_rx: UnboundedReceiver<Subscribe>,
    // Because the number of exchanges will be small, this would be faster as a `Vec`
    client_map: FxHashMap<ExchangeIdentifier, Box<dyn ExchangeClient>>,
    pair_map: FxHashMap<(ExchangeIdentifier, CurrencyPair), PairData>,
    subscriber_map: FxHashMap<usize, SubscriberData>,
    next_subscriber_index: usize,
}

impl AggregatorInner {
    fn new(ecf: Box<dyn ExchangeClientFactory>, sub_rx: UnboundedReceiver<Subscribe>) -> Self {
        Self {
            ecf,
            sub_rx,
            client_map: FxHashMap::default(),
            pair_map: FxHashMap::default(),
            subscriber_map: FxHashMap::default(),
            next_subscriber_index: 0,
        }
    }

    async fn run(&mut self) {
        loop {
            if self.pair_map.is_empty() {
                match self.sub_rx.recv().await {
                    Some(subscribe) => {
                        let res = self
                            .handle_subscribe(subscribe.exchange_idents, subscribe.currency_pair)
                            .await;
                        if subscribe.resp_tx.send(res).is_err() {
                            // TODO: Unsubscribe
                        }
                    }
                    None => return,
                }
            } else {
                // Aggregate
                let mut update_rx = self
                    .pair_map
                    .values_mut()
                    .enumerate()
                    .map(|(i, pair_data)| async move { (i, pair_data.summary_rx.recv().await) })
                    .collect::<FuturesUnordered<_>>();

                tokio::select! {
                    opt = update_rx.next() => {
                        drop(update_rx);

                        match opt {
                            Some((_, Some((exchange_ident, currency_pair, res)))) => {
                                self.handle_update(exchange_ident, currency_pair, res).await;
                            },
                            Some((_i, None)) => {
                                // tracing::warn!("AggregatorInner: an update_rx channel returned None");
                                // TODO: Handle this
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
                            None => {
                                tracing::warn!("AggregatorInner received None from sub_rx; returning");
                                return;
                            }
                        }
                    }
                };
            }
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

        //tracing::info!("Got {} update from {}", currency_pair, exchange_ident);

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
                Entry::Vacant(entry) => match self.ecf.make(*exchange_ident).await {
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

/// So Java! So we can test the aggregator with fake exchanges.
#[async_trait::async_trait]
pub trait ExchangeClientFactory: Send + Sync {
    async fn make(
        &mut self,
        exchange_ident: ExchangeIdentifier,
    ) -> Result<Box<dyn ExchangeClient>, ExchangeClientError>;
}

struct RealExchangeClientFactory;

#[async_trait::async_trait]
impl ExchangeClientFactory for RealExchangeClientFactory {
    async fn make(
        &mut self,
        exchange_ident: ExchangeIdentifier,
    ) -> Result<Box<dyn ExchangeClient>, ExchangeClientError> {
        Ok(match exchange_ident {
            ExchangeIdentifier::Binance => Box::new(BinanceClient),
            ExchangeIdentifier::Bitstamp => Box::new(BitstampClient::connect().await?),
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchanges::SummaryMessage;

    #[derive(Default)]
    struct FakeExchangeClient {
        summary_rx_map: FxHashMap<CurrencyPair, ExchangeSummaryReceiver>,
    }

    impl FakeExchangeClient {
        fn add_tx(&mut self, currency_pair: CurrencyPair) -> UnboundedSender<SummaryMessage> {
            let (summary_tx, summary_rx) = unbounded_channel();
            self.summary_rx_map.insert(currency_pair, summary_rx);
            summary_tx
        }
    }

    #[async_trait::async_trait]
    impl ExchangeClient for FakeExchangeClient {
        async fn get_summary_stream(
            &mut self,
            currency_pair: CurrencyPair,
        ) -> Result<ExchangeSummaryReceiver, ExchangeClientError> {
            Ok(self.summary_rx_map.remove(&currency_pair).expect(
                "Shouldn't call get_summary_stream more than once for the same currency pair",
            ))
        }
    }

    #[derive(Default)]
    struct TestExchangeClientFactory {
        client_map: FxHashMap<ExchangeIdentifier, FakeExchangeClient>,
    }

    impl TestExchangeClientFactory {
        fn add_client(&mut self, exchange_ident: ExchangeIdentifier, client: FakeExchangeClient) {
            self.client_map.insert(exchange_ident, client);
        }
    }

    #[async_trait::async_trait]
    impl ExchangeClientFactory for TestExchangeClientFactory {
        async fn make(
            &mut self,
            exchange_ident: ExchangeIdentifier,
        ) -> Result<Box<dyn ExchangeClient>, ExchangeClientError> {
            Ok(Box::new(self.client_map.remove(&exchange_ident).expect(
                "Shouldn't try connecting to the same exchange more than once",
            )))
        }
    }

    struct SummarySender {
        exchange_ident: ExchangeIdentifier,
        currency_pair: CurrencyPair,
        tx: UnboundedSender<(
            ExchangeIdentifier,
            CurrencyPair,
            Result<ob::Summary, ExchangeClientError>,
        )>,
    }

    impl SummarySender {
        fn send_ok(&self, summary: ob::Summary) {
            self.tx
                .send((self.exchange_ident, self.currency_pair, Ok(summary)))
                .unwrap();
        }
    }

    fn make_aggregator(
        exchange_idents: Vec<ExchangeIdentifier>,
        currency_pair: CurrencyPair,
    ) -> (Aggregator, Vec<SummarySender>) {
        let mut ecf = TestExchangeClientFactory::default();
        let mut summary_tx = vec![];

        exchange_idents.iter().for_each(|&exchange_ident| {
            let mut client = FakeExchangeClient::default();
            let tx = client.add_tx(currency_pair);

            ecf.add_client(exchange_ident, client);
            summary_tx.push(SummarySender {
                exchange_ident,
                currency_pair,
                tx,
            });
        });

        (Aggregator::new(Box::new(ecf)), summary_tx)
    }

    fn make_summary_one_exchange(
        exchange: &str,
        spread: f64,
        bids: &[(f64, f64)],
        asks: &[(f64, f64)],
    ) -> ob::Summary {
        ob::Summary {
            spread,
            bids: bids
                .into_iter()
                .map(|&(price, amount)| ob::Level {
                    exchange: exchange.into(),
                    price,
                    amount,
                })
                .collect(),
            asks: asks
                .into_iter()
                .map(|&(price, amount)| ob::Level {
                    exchange: exchange.into(),
                    price,
                    amount,
                })
                .collect(),
        }
    }

    fn make_summary_multi_exchange(
        spread: f64,
        bids: &[(f64, f64, &str)],
        asks: &[(f64, f64, &str)],
    ) -> ob::Summary {
        ob::Summary {
            spread,
            bids: bids
                .into_iter()
                .map(|&(price, amount, exchange)| ob::Level {
                    exchange: exchange.into(),
                    price,
                    amount,
                })
                .collect(),
            asks: asks
                .into_iter()
                .map(|&(price, amount, exchange)| ob::Level {
                    exchange: exchange.into(),
                    price,
                    amount,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn test_orderbook_aggregation() {
        // Just show that basic aggregation works and that the aggregator
        // caches state correctly for each exchange.

        let (aggregator, mut summary_tx) = make_aggregator(
            vec![ExchangeIdentifier::Binance, ExchangeIdentifier::Bitstamp],
            CurrencyPair::BtcUsd,
        );

        let (binance_tx, bitstamp_tx) = {
            let mut summary_tx = summary_tx.drain(..);
            let binance_tx = summary_tx.next().unwrap();
            let bitstamp_tx = summary_tx.next().unwrap();
            (binance_tx, bitstamp_tx)
        };

        // Look at Binance only
        let mut agg_rx = aggregator
            .get_summary_stream(
                vec![ExchangeIdentifier::Binance, ExchangeIdentifier::Bitstamp],
                CurrencyPair::BtcUsd,
            )
            .await
            .unwrap();

        // Make sure there is an initial update (with an empty orderbook in this case)
        assert_eq!(
            agg_rx.recv().await.unwrap(),
            ob::Summary {
                spread: 0.0,
                bids: vec![],
                asks: vec![]
            }
        );

        // There shouldn't be another update
        assert!(agg_rx.try_recv().is_err());

        // Now we send an update from "Binance"
        binance_tx.send_ok(make_summary_one_exchange(
            "binance",
            0.0, // Spread is recalculated by aggregator
            &[(100.0, 10.0), (99.0, 10.0)],
            &[(105.0, 10.0), (106.0, 10.0)],
        ));

        // We should receive this from the aggregator
        assert_eq!(
            agg_rx.recv().await.unwrap(),
            make_summary_one_exchange(
                "binance",
                5.0, // Spread is recalculated by aggregator
                &[(100.0, 10.0), (99.0, 10.0)],
                &[(105.0, 10.0), (106.0, 10.0)],
            )
        );

        // Now we send an update from "Bitstamp"
        bitstamp_tx.send_ok(make_summary_one_exchange(
            "bitstamp",
            0.0, // Spread is recalculated by aggregator
            &[(99.5, 10.0), (98.5, 10.0)],
            &[(105.5, 10.0), (106.5, 10.0)],
        ));

        // We should now get the interleaved values from the aggregator
        assert_eq!(
            agg_rx.recv().await.unwrap(),
            make_summary_multi_exchange(
                5.0, // Spread is recalculated by aggregator
                &[
                    (100.0, 10.0, "binance"),
                    (99.5, 10.0, "bitstamp"),
                    (99.0, 10.0, "binance"),
                    (98.5, 10.0, "bitstamp")
                ],
                &[
                    (105.0, 10.0, "binance"),
                    (105.5, 10.0, "bitstamp"),
                    (106.0, 10.0, "binance"),
                    (106.5, 10.0, "bitstamp")
                ],
            )
        );
    }
}
