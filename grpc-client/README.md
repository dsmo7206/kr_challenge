# grpc-client

This is a simple command-line application that connects to the server and prints a stream of orderbook updates.

If run without additional args, it will print the ETH/BTC stream aggregated from Binance and Bitstamp.

It can also be run with args to call the alternate "custom" gRPC endpoint which allows specification of currency pair and a list of exchanges to aggregate over.

I haven't implemented any other exchange connections, but I did add a couple of different currency pairs for testing. To try one of these:

(from the project root)
`cargo run --bin grpc-client --release -- -p BTC/GBP -e binance`
to look at BTC/GBP over Binance only (i.e. no aggregation with Bitstamp)
