use super::aggregator::Aggregator;
use common::{CurrencyPair, ExchangeIdentifier};
use futures::Stream;
use orderbook as ob;
use std::{net::SocketAddr, pin::Pin, str::FromStr};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    transport::{Error as TonicError, Server as TonicServer},
    Code, Request, Response, Status,
};

type Server = ob::OrderbookAggregatorServer<MyOrderbookAggregator>;

type SummaryResponseStream = Pin<Box<dyn Stream<Item = Result<ob::Summary, Status>> + Send>>;

pub async fn serve_forever(port: u16, aggregator: Aggregator) -> Result<(), TonicError> {
    let addr: SocketAddr = format!("[::1]:{port}").parse().unwrap();

    TonicServer::builder()
        .add_service(Server::new(MyOrderbookAggregator::new(aggregator)))
        .serve(addr)
        .await
}

#[derive(Debug)]
struct MyOrderbookAggregator {
    aggregator: Aggregator,
}

impl MyOrderbookAggregator {
    fn new(aggregator: Aggregator) -> Self {
        Self { aggregator }
    }
}

#[tonic::async_trait]
impl ob::OrderbookAggregator for MyOrderbookAggregator {
    type BookSummaryStream = SummaryResponseStream;
    type BookSummaryCustomStream = SummaryResponseStream;

    async fn book_summary(
        &self,
        _: Request<ob::Empty>,
    ) -> Result<Response<Self::BookSummaryStream>, Status> {
        let mut summary_stream = self
            .aggregator
            .get_summary_stream(
                vec![ExchangeIdentifier::Binance, ExchangeIdentifier::Bitstamp],
                CurrencyPair::EthBtc,
            )
            .await
            .map_err(|err| Status::new(Code::Internal, err.to_string()))?;

        let (tx, rx) = mpsc::channel(128);

        tokio::spawn(async move {
            while let Some(summary) = summary_stream.recv().await {
                match tx.send(Result::<_, Status>::Ok(summary)).await {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as Self::BookSummaryStream
        ))
    }

    async fn book_summary_custom(
        &self,
        req: Request<ob::CustomArgs>,
    ) -> Result<Response<Self::BookSummaryStream>, Status> {
        // Parse arguments
        let args = req.into_inner();

        let Ok(currency_pair) = CurrencyPair::from_str(&args.name) else {
            return Err(Status::new(Code::InvalidArgument, "Invalid currency pair"));
        };

        let exchange_idents: Result<Vec<_>, _> = args
            .exchange_idents
            .into_iter()
            .map(|exchange_ident| ExchangeIdentifier::from_str(&exchange_ident))
            .collect();

        let Ok(exchange_idents) = exchange_idents else {
            return Err(Status::new(Code::InvalidArgument, "At least one invalid exchange identifier"));
        };

        let mut summary_stream = self
            .aggregator
            .get_summary_stream(exchange_idents, currency_pair)
            .await
            .map_err(|err| Status::new(Code::Internal, err.to_string()))?;

        let (tx, rx) = mpsc::channel(128);

        tokio::spawn(async move {
            while let Some(summary) = summary_stream.recv().await {
                match tx.send(Result::<_, Status>::Ok(summary)).await {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as Self::BookSummaryStream
        ))
    }
}
