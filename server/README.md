# server

This is the main gRPC server that connects to the exchanges and sends out summary streams.

It seems likely that Binance and Bitstamp wouldn't be the only two exchanges in a real system, so I hid their implementation details behind the `ExchangeClient` interface so it's easier to add more.

The `Aggregator` is the core struct - it can return summary streams for a given currency pair across any number of exchanges (even though there are only two implemented at the moment).

The `Aggregator` tries to be slightly efficient - it has a cache of last-received orderbook updates for each (exchange, currency_pair) pair, and only subscribes once to each exchange for what it needs, which presumably would lower subscription costs (where they exist) in the real world

What the `Aggregator` doesn't try to do is optimise the actual aggregation when an update comes in. If one summary aggregation was calculated across exchanges (E0, E1, E2, E3, E4) and another across (E0, E1, E2, E3, E4, E5), you could imagine optimising this by interleaving E5's orderbook with the [E0..E4] aggregate, but I haven't bothered to do this.

Also, exchange connections aren't being dropped when the last summary is unsubscribed from, nor are all error conditions being handled properly, just because of time constraints.

Instead of having explicit "unsubscribe" messages, I'm relying on the `Err(...)` being returned by the `send`/`recv` on the channels used to signify "unsubscribing", but I'm not paying attention to the results everywhere. Sometimes this means that objects aren't cleaned up until the next message comes in from a channel, which isn't terrible but also probably isn't ideal.
