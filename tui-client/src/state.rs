use common::{CurrencyPair, ExchangeIdentifier};
use orderbook as ob;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, MutexGuard};

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

const MAX_LOG_MESSAGES: usize = 10;

pub fn get_should_stop() -> bool {
    SHOULD_STOP.load(Ordering::Relaxed)
}

pub fn set_should_stop() {
    SHOULD_STOP.store(true, Ordering::Relaxed);
}

pub async fn lock_recursive(state: &State) -> LockedState {
    let mut inner = vec![];
    for ob in state.orderbooks.iter() {
        inner.push(LockedOrderbookState {
            title: &ob.title,
            summary: ob.summary.lock().await,
        });
    }

    LockedState {
        is_connected: state.grpc_client.is_some(),
        input_text: &state.input_text,
        log_messages: &state.log_messages,
        orderbooks: inner,
        selected_tab_index: state.selected_tab_index,
    }
}

pub struct State {
    pub grpc_client: Option<ob::OrderbookAggregatorClient<tonic::transport::channel::Channel>>,
    pub input_text: String,
    pub log_messages: VecDeque<String>,
    pub orderbooks: Vec<OrderbookState>,
    pub selected_tab_index: Option<usize>,
}

impl State {
    pub fn new() -> State {
        State {
            grpc_client: None,
            input_text: String::from(""),
            log_messages: VecDeque::with_capacity(MAX_LOG_MESSAGES),
            orderbooks: vec![],
            selected_tab_index: None,
        }
    }

    pub fn add_log_message(&mut self, msg: String) {
        if self.log_messages.len() == MAX_LOG_MESSAGES {
            self.log_messages.pop_front();
        }
        self.log_messages.push_back(msg);
    }
}

pub struct LockedState<'a> {
    pub is_connected: bool,
    pub input_text: &'a str,
    pub log_messages: &'a VecDeque<String>,
    pub orderbooks: Vec<LockedOrderbookState<'a>>,
    pub selected_tab_index: Option<usize>,
}

pub struct OrderbookState {
    pub title: String,
    pub currency_pair: CurrencyPair,
    pub exchange_idents: Vec<ExchangeIdentifier>,
    pub summary: Arc<Mutex<ob::Summary>>,
}

impl OrderbookState {
    pub fn new(
        currency_pair: CurrencyPair,
        exchange_idents: Vec<ExchangeIdentifier>,
    ) -> OrderbookState {
        let mut title = format!("{} on {}", currency_pair, exchange_idents[0]);
        for exchange_ident in &exchange_idents[1..] {
            title.push_str(&format!("+{}", exchange_ident));
        }

        OrderbookState {
            title,
            currency_pair,
            exchange_idents,
            summary: Arc::new(Mutex::new(ob::Summary {
                spread: 0.0,
                bids: vec![],
                asks: vec![],
            })),
        }
    }
}

pub struct LockedOrderbookState<'a> {
    pub title: &'a str,
    pub summary: MutexGuard<'a, ob::Summary>,
}
