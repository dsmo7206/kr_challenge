// Re-export compiled types
pub use crate::orderbook::{
    orderbook_aggregator_client::OrderbookAggregatorClient,
    orderbook_aggregator_server::{OrderbookAggregator, OrderbookAggregatorServer},
    CustomArgs, Empty, Level, Summary,
};

mod orderbook {
    tonic::include_proto!("orderbook");
}
