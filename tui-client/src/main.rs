mod command;
mod events;
mod ui;
mod ui_state;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use events::{get_next_key_press, handle_events};
use std::{error::Error, io, sync::Arc, time::Duration};
use tokio::sync::{mpsc::unbounded_channel, Mutex};
use tui::{backend::CrosstermBackend, Terminal};
use ui::draw_ui;
use ui_state::{get_should_stop, UIState};

const KEY_POLL_TIMEOUT: Duration = Duration::from_millis(50);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let state = Arc::new(Mutex::new(UIState::new()));
    let state_clone = state.clone();

    let (key_tx, key_rx) = unbounded_channel();

    tokio::spawn(async move {
        handle_events(state_clone, key_rx).await;
    });

    // Run loop
    loop {
        if get_should_stop() {
            break;
        }

        let mut state_guard = state.lock().await;

        terminal.draw(|f| draw_ui(f, &mut state_guard)).unwrap();

        // This will sleep for up to `KEY_POLL_TIMEOUT` ms.

        let key_join_handle =
            tokio::task::spawn_blocking(move || get_next_key_press(KEY_POLL_TIMEOUT));

        if let Ok(Some(key_event)) = key_join_handle.await {
            key_tx.send(key_event).ok();
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
