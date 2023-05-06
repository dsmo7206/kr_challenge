use super::{
    command::Command,
    state::{get_should_stop, set_should_stop, OrderbookState, State},
};
use crossterm::event::{poll as poll_key, read as read_event, Event, KeyCode, KeyEvent};
use futures::StreamExt;
use orderbook as ob;
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::sync::{mpsc::unbounded_channel, Mutex, MutexGuard};

const KEY_POLL_TIMEOUT: Duration = Duration::from_millis(100);

pub async fn handle_events(state: Arc<Mutex<State>>) {
    let (key_tx, mut key_rx) = unbounded_channel();

    tokio::task::spawn_blocking(move || loop {
        if get_should_stop() {
            break;
        }

        if let Some(key_event) = get_next_key_press() {
            key_tx.send(key_event).unwrap();
        }
    });

    loop {
        if get_should_stop() {
            break;
        }

        tokio::select! {
            key_event = key_rx.recv() => {
                if let Some(key_event) = key_event {
                    handle_key_event(&mut state.lock().await, key_event).await;
                }
            }
        }
    }
}

/// Takes up to `KEY_POLL_TIMEOUT` to run
fn get_next_key_press() -> Option<KeyEvent> {
    match poll_key(KEY_POLL_TIMEOUT) {
        Ok(true) => {
            if let Event::Key(key) = read_event().unwrap() {
                Some(key)
            } else {
                None
            }
        }
        _ => None,
    }
}

async fn handle_key_event(state: &mut MutexGuard<'_, State>, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Char(c) => {
            state.input_text.push(c);
        }
        KeyCode::Backspace => {
            state.input_text.pop();
        }
        KeyCode::Enter => {
            let input_text = std::mem::take(&mut state.input_text);

            let cmd = match Command::from_str(&input_text) {
                Ok(cmd) => cmd,
                Err(err) => {
                    state.add_log_message(err);
                    return;
                }
            };

            if let Err(err) = handle_cmd(state, cmd).await {
                state.add_log_message(format!("ERROR: {err}"));
            }
        }
        KeyCode::Esc => {
            set_should_stop();
        }
        _ => {}
    }
}

async fn handle_cmd(state: &mut MutexGuard<'_, State>, cmd: Command) -> Result<(), String> {
    match cmd {
        Command::Connect(server_addr) => {
            state.grpc_client = Some(grpc_connect(&server_addr).await?);
            state.orderbooks.clear();
            state.selected_tab_index = None;
            state.add_log_message(format!("Connected to {}!", server_addr));
            Ok(())
        }
        Command::Open(currency_pair, exchange_idents) => match &mut state.grpc_client {
            Some(grpc_client) => {
                let mut stream = grpc_client
                    .book_summary_custom(ob::CustomArgs {
                        name: currency_pair.to_string(),
                        exchange_idents: exchange_idents
                            .iter()
                            .map(|ident| ident.to_string())
                            .collect(),
                    })
                    .await
                    .map(|resp| resp.into_inner())
                    .map_err(|err| format!("Could not connect: {err}"))?;

                let new_tab_index = state.orderbooks.len();
                state.selected_tab_index = Some(new_tab_index);

                let obs = OrderbookState::new(currency_pair, exchange_idents);
                // Grab an Arc to this for modification inside coroutine
                let summary_clone = obs.summary.clone();

                state.orderbooks.push(obs);

                tokio::spawn(async move {
                    loop {
                        if get_should_stop() {
                            break;
                        }

                        if Arc::strong_count(&summary_clone) == 1 {
                            // This is so gross...
                            break;
                        }

                        tokio::select! {
                            opt = stream.next() => {
                                if let Some(Ok(summary)) = opt {
                                    // Ignore errors
                                    *summary_clone.lock().await = summary;
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                        }
                    }
                });

                state.add_log_message("Opened new channel!".to_string());

                Ok(())
            }
            None => Err("ERROR: Not connected to server".to_string()),
        },
        Command::ShowTab(index) => {
            if state.orderbooks.is_empty() {
                Err("You need to run the \"open\" command first!".into())
            } else if index >= state.orderbooks.len() {
                Err(format!(
                    "Invalid index (max: {})",
                    state.orderbooks.len() - 1
                ))
            } else {
                state.selected_tab_index = Some(index);
                state.add_log_message(format!("Showing tab {}!", index));

                Ok(())
            }
        }
        Command::CloseTab(index) => {
            if state.orderbooks.is_empty() {
                Err("There are no tabs to close!".into())
            } else if index >= state.orderbooks.len() {
                Err(format!(
                    "Invalid index (max: {})",
                    state.orderbooks.len() - 1
                ))
            } else {
                state.orderbooks.remove(index);

                if state.orderbooks.is_empty() {
                    state.selected_tab_index = None;
                } else {
                    state.selected_tab_index = Some(0);
                }
                state.add_log_message(format!("Tab {} closed!", index));

                Ok(())
            }
        }
        Command::Quit => {
            set_should_stop();
            Ok(())
        }
    }
}

async fn grpc_connect(
    server_addr: &str,
) -> Result<ob::OrderbookAggregatorClient<tonic::transport::channel::Channel>, &'static str> {
    ob::OrderbookAggregatorClient::connect(server_addr.to_owned())
        .await
        .map_err(|_| "Could not connect")
}
