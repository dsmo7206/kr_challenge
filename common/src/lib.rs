use strum_macros::{Display, EnumString};

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
pub enum CurrencyPair {
    #[strum(serialize = "BTC/GBP")]
    BtcGbp,
    #[strum(serialize = "BTC/USD")]
    BtcUsd,
    #[strum(serialize = "ETH/BTC")]
    EthBtc,
}

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
pub enum ExchangeIdentifier {
    #[strum(serialize = "binance")]
    Binance,
    #[strum(serialize = "bitstamp")]
    Bitstamp,
}
