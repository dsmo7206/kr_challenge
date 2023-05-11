mod aggregator;
mod config;
mod exchanges;
mod grpc_server;

use aggregator::Aggregator;
use config::Config;
use envconfig::Envconfig;
use grpc_server::serve_forever as serve_grpc_forever;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cfg = Config::init_from_env()?;

    serve_grpc_forever(cfg.grpc_port, Aggregator::default())
        .await
        .map_err(Into::into)
}
