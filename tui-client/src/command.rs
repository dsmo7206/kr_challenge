use common::{CurrencyPair, ExchangeIdentifier};

const ERR_CURRENCY_PAIR: &str = "Invalid currency pair";
const ERR_USAGE_CONNECT: &str = "Usage: connect <server_addr>";
const ERR_USAGE_OPEN: &str = "Usage: open <currency_pair> <exchange>+";
const ERR_USAGE_SHOW: &str = "Usage: show <tab_index>";
const ERR_USAGE_CLOSE: &str = "Usage: close <tab_index>";
const ERR_INVALID_COMMAND: &str = "Invalid command";

pub enum Command {
    Connect(String),
    Open(CurrencyPair, Vec<ExchangeIdentifier>),
    ShowTab(usize),
    CloseTab(usize),
    Help,
    Quit,
}

impl std::str::FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "" => return Err("Enter a command".into()),
            "help" => return Ok(Command::Help),
            "quit" => return Ok(Command::Quit),
            _ => {}
        };

        let parts = s.split_ascii_whitespace().collect::<Vec<_>>();

        match parts[0] {
            "connect" => {
                if parts.len() == 2 {
                    Ok(Command::Connect(parts[1].into()))
                } else {
                    Err(ERR_USAGE_CONNECT.into())
                }
            }
            "open" => {
                let mut rest_parts = parts.into_iter().skip(1);

                let currency_pair_str = rest_parts.next().ok_or(ERR_USAGE_OPEN)?;
                let currency_pair = CurrencyPair::from_str(currency_pair_str)
                    .map_err(|_| ERR_CURRENCY_PAIR.to_owned())?;

                let mut exchange_idents = vec![rest_parts.next().ok_or(ERR_USAGE_OPEN)?];
                exchange_idents.extend(rest_parts);

                let exchange_idents: Result<Vec<_>, _> = exchange_idents
                    .into_iter()
                    .map(ExchangeIdentifier::from_str)
                    .collect();

                Ok(Command::Open(
                    currency_pair,
                    exchange_idents.map_err(|err| format!("Invalid exchange ident: {err}"))?,
                ))
            }
            "show" => {
                if parts.len() == 2 {
                    Ok(Command::ShowTab(
                        parts[1].parse().map_err(|_| ERR_USAGE_SHOW.to_owned())?,
                    ))
                } else {
                    Err(ERR_USAGE_SHOW.into())
                }
            }
            "close" => {
                if parts.len() == 2 {
                    Ok(Command::CloseTab(
                        parts[1].parse().map_err(|_| ERR_USAGE_CLOSE.to_owned())?,
                    ))
                } else {
                    Err(ERR_USAGE_CLOSE.into())
                }
            }
            _ => Err(ERR_INVALID_COMMAND.into()),
        }
    }
}
