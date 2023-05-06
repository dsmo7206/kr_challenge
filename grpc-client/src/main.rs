mod config;

use clap::Parser;
use config::Config;
use orderbook as ob;
use tokio_stream::StreamExt;
use tonic::Streaming;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::parse();

    let mut client = ob::OrderbookAggregatorClient::connect(cfg.server_addr).await?;

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
        _ => panic!(),
    }?
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
