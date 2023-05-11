mod config;

use clap::Parser;
use config::Config;
use orderbook as ob;
use std::process::{ExitCode, Termination};
use tokio_stream::StreamExt;
use tonic::{Status, Streaming};

#[derive(Debug)]
enum MainError {
    InconsistentArgs,
    ServerConnectError(String),
    StreamError(Status),
}

impl Termination for MainError {
    fn report(self) -> ExitCode {
        match self {
            Self::InconsistentArgs => {
                println!("If specifying either currency_pair or market(s), both must be specified")
            }
            Self::ServerConnectError(server_addr) => {
                println!("Could not connect to server at {}", server_addr)
            }
            Self::StreamError(status) => {
                println!("Problem connecting to gRPC stream: {}", status);
            }
        }
        ExitCode::FAILURE
    }
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    let cfg = Config::parse();

    let mut client = ob::OrderbookAggregatorClient::connect(cfg.server_addr.clone())
        .await
        .map_err(|_| MainError::ServerConnectError(cfg.server_addr))?;

    let stream = match (cfg.exchange_ident, cfg.pair_name) {
        (Some(exchange_idents), Some(pair_name)) => {
            client
                .book_summary_custom(ob::CustomArgs {
                    name: pair_name,
                    exchange_idents,
                })
                .await
        }
        (None, None) => {
            // The example as provided in the document
            client.book_summary(ob::Empty {}).await
        }
        _ => return Err(MainError::InconsistentArgs),
    }
    .map_err(MainError::StreamError)?
    .into_inner();

    print_stream(stream).await;

    Ok(())
}

async fn print_stream(mut stream: Streaming<ob::Summary>) {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(summary) => {
                println!("Received OK: {:?}", summary);
            }
            Err(status) => {
                println!("Received error status: {}", status);
            }
        }
    }
}
