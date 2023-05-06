mod binance;
mod bitstamp;

use common::{CurrencyPair, ExchangeIdentifier};
use orderbook as ob;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub use binance::Client as BinanceClient;
pub use bitstamp::Client as BitstampClient;

#[async_trait::async_trait]
pub trait ExchangeClient: Send + Sync {
    async fn get_summary_stream(
        &mut self,
        currency_pair: CurrencyPair,
    ) -> Result<SummaryReceiver, ExchangeClientError>;
}

#[derive(Debug)]
pub enum ExchangeClientError {
    Connect(Box<dyn std::error::Error + Send>),
    _InvalidCurrencyPair(CurrencyPair),
    InvalidDecimal,
}

impl std::fmt::Display for ExchangeClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(err) => write!(f, "Error connecting: {}", err),
            Self::_InvalidCurrencyPair(currency_pair) => {
                write!(f, "Invalid currency pair: {}", currency_pair)
            }
            Self::InvalidDecimal => write!(f, "Invalid decimal"),
        }
    }
}

impl std::error::Error for ExchangeClientError {}

pub type SummaryMessage = (
    ExchangeIdentifier,
    CurrencyPair,
    Result<ob::Summary, ExchangeClientError>,
);
pub type SummarySender = UnboundedSender<SummaryMessage>;
pub type SummaryReceiver = UnboundedReceiver<SummaryMessage>;
