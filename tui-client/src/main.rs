mod command;
mod events;
mod state;
mod ui;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use events::handle_events;
use state::{get_should_stop, lock_recursive, State};
use std::{error::Error, io, sync::Arc};
use tokio::sync::Mutex;
use tui::{backend::CrosstermBackend, Terminal};
use ui::draw_ui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let state = Arc::new(Mutex::new(State::new()));
    let state_clone = state.clone();

    tokio::spawn(async move {
        handle_events(state_clone).await;
    });

    // Run loop
    loop {
        let state_guard = state.lock().await;
        let state = lock_recursive(&state_guard).await;

        if get_should_stop() {
            break;
        }

        terminal.draw(|f| draw_ui(f, state)).unwrap();
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
