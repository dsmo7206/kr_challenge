# KR Challenge

To start the server:

`cargo run --bin server --release`

To run the "gRPC client" that prints a stream of orderbook updates for ETH/BTC across Binance and Bitstamp (not formatted nicely at all!):

`cargo run --bin grpc-client --release`

To run the (not asked for but fun to write) TUI (text UI) client:

`cargo run --bin tui-client --release`

For more information please see the `README.md` in each crate.
