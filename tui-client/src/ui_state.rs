use common::{CurrencyPair, ExchangeIdentifier};
use orderbook as ob;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tui::widgets::{ListState, TableState};

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

const MAX_LOG_MESSAGES: usize = 10;
pub const RECENT_MID_PRICE_WINDOW_SECS: u64 = 20;

pub fn get_should_stop() -> bool {
    SHOULD_STOP.load(Ordering::Relaxed)
}

pub fn set_should_stop() {
    SHOULD_STOP.store(true, Ordering::Relaxed);
}

pub struct UIState {
    pub is_connected: bool,
    pub input_text: String,
    pub log_messages: VecDeque<(usize, String)>,
    pub log_messages_list_state: ListState,
    pub next_log_message_index: usize,
    pub orderbooks: Vec<OrderbookState>,
    pub selected_tab_index: Option<usize>,
    pub show_help_popup: bool,
}

impl UIState {
    pub fn new() -> UIState {
        UIState {
            is_connected: false,
            input_text: String::from(""),
            log_messages: VecDeque::with_capacity(MAX_LOG_MESSAGES),
            log_messages_list_state: ListState::default(),
            next_log_message_index: 0,
            orderbooks: vec![],
            selected_tab_index: None,
            show_help_popup: false,
        }
    }

    pub fn add_log_message(&mut self, msg: String) {
        if self.log_messages.len() == MAX_LOG_MESSAGES {
            self.log_messages.pop_front();
        }

        self.log_messages
            .push_back((self.next_log_message_index, msg));

        self.next_log_message_index += 1;
    }
}

pub struct OrderbookState {
    pub title: String,
    pub currency_pair: CurrencyPair,
    pub exchange_idents: Vec<ExchangeIdentifier>,
    pub summary: ob::Summary,
    pub recent_mid_prices: VecDeque<(f64, f64)>,
    pub min_price_seen: Option<f64>,
    pub max_price_seen: Option<f64>,
    pub table_state: TableState,
    pub start_time: Instant,
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
            summary: ob::Summary {
                spread: 0.0,
                bids: vec![],
                asks: vec![],
            },
            recent_mid_prices: VecDeque::with_capacity(100),
            min_price_seen: None,
            max_price_seen: None,
            table_state: TableState::default(),
            start_time: Instant::now(),
        }
    }

    pub fn update_summary(&mut self, summary: ob::Summary) {
        let new_mid_price = match (summary.bids.first(), summary.asks.first()) {
            (Some(bid), Some(ask)) => Some(0.5 * (bid.price + ask.price)),
            (Some(bid), None) => Some(bid.price),
            (None, Some(ask)) => Some(ask.price),
            (None, None) => None,
        };

        if let Some(new_mid_price) = new_mid_price {
            // Define the new window start we want
            let cull_time = Instant::now() - Duration::from_secs(RECENT_MID_PRICE_WINDOW_SECS);
            let cull_time_ts = (cull_time - self.start_time).as_secs_f64();

            // Cull the window
            while let Some(&(first_time, _)) = self.recent_mid_prices.front() {
                if first_time < cull_time_ts {
                    self.recent_mid_prices.pop_front();
                } else {
                    break;
                }
            }

            self.recent_mid_prices
                .push_back((self.start_time.elapsed().as_secs_f64(), new_mid_price));

            self.min_price_seen = Some(match self.min_price_seen {
                Some(prev_min) => prev_min.min(new_mid_price),
                None => new_mid_price,
            });

            self.max_price_seen = Some(match self.max_price_seen {
                Some(prev_max) => prev_max.max(new_mid_price),
                None => new_mid_price,
            });
        }

        self.summary = summary;
    }
}
