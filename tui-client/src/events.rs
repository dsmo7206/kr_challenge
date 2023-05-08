use super::{
    command::Command,
    ui_state::{get_should_stop, set_should_stop, OrderbookState, UIState},
};
use crossterm::event::{poll as poll_key, read as read_event, Event, KeyCode, KeyEvent};
use futures::{stream::FuturesUnordered, StreamExt};
use orderbook as ob;
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::sync::{mpsc::UnboundedReceiver, Mutex};
use tonic::Streaming;

struct EventsState {
    grpc_client: Option<ob::OrderbookAggregatorClient<tonic::transport::channel::Channel>>,
    streams: Vec<Streaming<ob::Summary>>,
}

pub async fn handle_events(
    mut ui_state: Arc<Mutex<UIState>>,
    mut key_rx: UnboundedReceiver<KeyEvent>,
) {
    let mut events_state = EventsState {
        grpc_client: None,
        streams: vec![],
    };

    loop {
        if get_should_stop() {
            break;
        }

        if events_state.streams.is_empty() {
            if let Some(key_event) = key_rx.recv().await {
                handle_key_event(&mut ui_state, &mut events_state, key_event).await;
            }
        } else {
            let mut streams_rx = events_state
                .streams
                .iter_mut()
                .enumerate()
                .map(|(i, stream)| async move { (i, stream.next().await) })
                .collect::<FuturesUnordered<_>>();

            tokio::select! {
                key_event = key_rx.recv() => {
                    drop(streams_rx); // Release events_state reference

                    if let Some(key_event) = key_event {
                        handle_key_event(&mut ui_state, &mut events_state, key_event).await;
                    }
                }
                stream_event = streams_rx.next() => {
                    if let Some((i, stream_event)) = stream_event {
                        match stream_event {
                            Some(Ok(summary)) => {
                                ui_state.lock().await.orderbooks[i].update_summary(summary);
                            },
                            Some(Err(status)) => {
                                ui_state.lock().await.add_log_message(format!("Error from stream {i}: {status}"));
                            }
                            None => {
                                // Connection dropped
                                drop(streams_rx);

                                events_state.streams.remove(i);
                                ui_state.lock().await.orderbooks.remove(i);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Takes up to `KEY_POLL_TIMEOUT` to run
pub fn get_next_key_press(timeout: Duration) -> Option<KeyEvent> {
    match poll_key(timeout) {
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

async fn handle_key_event(
    ui_state: &mut Arc<Mutex<UIState>>,
    events_state: &mut EventsState,
    key_event: KeyEvent,
) {
    match key_event.code {
        KeyCode::Char(c) => {
            ui_state.lock().await.input_text.push(c);
        }
        KeyCode::Backspace => {
            ui_state.lock().await.input_text.pop();
        }
        KeyCode::Enter => {
            let input_text = std::mem::take(&mut ui_state.lock().await.input_text);

            let cmd = match Command::from_str(&input_text) {
                Ok(cmd) => cmd,
                Err(err) => {
                    ui_state.lock().await.add_log_message(err);
                    return;
                }
            };

            if let Err(err) = handle_cmd(ui_state, events_state, cmd).await {
                ui_state
                    .lock()
                    .await
                    .add_log_message(format!("ERROR: {err}"));
            }
        }
        KeyCode::Esc => {
            set_should_stop();
        }
        _ => {}
    }
}

async fn handle_cmd(
    ui_state: &mut Arc<Mutex<UIState>>,
    events_state: &mut EventsState,
    cmd: Command,
) -> Result<(), String> {
    match cmd {
        Command::Connect(server_addr) => {
            events_state.grpc_client = Some(grpc_connect(&server_addr).await?);

            let mut ui_state = ui_state.lock().await;
            ui_state.orderbooks.clear();
            ui_state.selected_tab_index = None;
            ui_state.add_log_message(format!("Connected to {}!", server_addr));

            Ok(())
        }
        Command::Open(currency_pair, exchange_idents) => match &mut events_state.grpc_client {
            Some(grpc_client) => {
                let stream = grpc_client
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

                events_state.streams.push(stream);

                {
                    // Update UI
                    let mut ui_state = ui_state.lock().await;
                    let new_tab_index = ui_state.orderbooks.len();
                    ui_state.selected_tab_index = Some(new_tab_index);
                    ui_state
                        .orderbooks
                        .push(OrderbookState::new(currency_pair, exchange_idents));
                    ui_state.add_log_message("Opened new channel!".to_string());
                }

                Ok(())
            }
            None => Err("ERROR: Not connected to server".to_string()),
        },
        Command::ShowTab(index) => {
            let mut ui_state = ui_state.lock().await;

            if ui_state.orderbooks.is_empty() {
                Err("You need to run the \"open\" command first!".into())
            } else if index >= ui_state.orderbooks.len() {
                Err(format!(
                    "Invalid index (max: {})",
                    ui_state.orderbooks.len() - 1
                ))
            } else {
                ui_state.selected_tab_index = Some(index);
                ui_state.add_log_message(format!("Showing tab {}!", index));

                Ok(())
            }
        }
        Command::CloseTab(index) => {
            let mut ui_state = ui_state.lock().await;

            if ui_state.orderbooks.is_empty() {
                Err("There are no tabs to close!".into())
            } else if index >= ui_state.orderbooks.len() {
                Err(format!(
                    "Invalid index (max: {})",
                    ui_state.orderbooks.len() - 1
                ))
            } else {
                ui_state.orderbooks.remove(index);
                events_state.streams.remove(index);

                if ui_state.orderbooks.is_empty() {
                    ui_state.selected_tab_index = None;
                } else {
                    ui_state.selected_tab_index = Some(0);
                }
                ui_state.add_log_message(format!("Tab {} closed!", index));

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
