use super::state::LockedState;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Tabs},
    Frame,
};

pub fn draw_ui<B>(f: &mut Frame<B>, state: LockedState)
where
    B: Backend,
{
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Min(1),
                Constraint::Length(3),
                Constraint::Length(12),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.size());

    if state.orderbooks.is_empty() {
        f.render_widget(make_main_empty(), chunks[0]);
    } else {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
            .split(chunks[0]);

        f.render_widget(make_main_tabs(&state), main_chunks[0]);
        f.render_widget(make_table(&state), main_chunks[1]);
    }

    f.render_widget(make_input(&state), chunks[1]);
    f.render_widget(make_log_messages(&state), chunks[2]);
    f.render_widget(make_status(&state), chunks[3]);

    f.set_cursor(
        // Put cursor past the end of the input text
        chunks[1].x + state.input_text.len() as u16 + 1,
        // Move one line down, from the border to the input line
        chunks[1].y + 1,
    );
}

fn make_main_empty() -> Paragraph<'static> {
    Paragraph::new(
        "Welcome to the TUI Client!

To get started, try the following inputs:

connect http://[::1]:50051
open ETH/BTC binance bitstamp",
    )
    .style(Style::default().fg(Color::Yellow))
    // .block(
    //     Block::default()
    //         .borders(Borders::ALL)
    //         .title(" Input (type \"help\" for help, \"quit\" to quit) "),
    // )
}

fn make_main_tabs<'a>(state: &'a LockedState) -> Tabs<'a> {
    let titles = state
        .orderbooks
        .iter()
        .map(|ob| &ob.title)
        .map(|title| {
            let (first, rest) = title.split_at(1);
            Spans::from(vec![
                Span::styled(first, Style::default().fg(Color::Yellow)),
                Span::styled(rest, Style::default().fg(Color::Green)),
            ])
        })
        .collect();

    Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Tabs"))
        .select(state.selected_tab_index.unwrap())
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Black),
        )
}

fn make_table<'a>(state: &'a LockedState) -> Table<'a> {
    let normal_style = Style::default().bg(Color::Blue);
    let header_cells = [
        "Pos",
        "Bid Price",
        "Bid Volume",
        "Bid Source",
        "Ask Price",
        "Ask Volume",
        "Ask Source",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
    let header = Row::new(header_cells)
        .style(normal_style)
        .height(1)
        .bottom_margin(1);

    let orderbook = &state.orderbooks[state.selected_tab_index.unwrap()];

    let rows = orderbook
        .summary
        .bids
        .iter()
        .zip(orderbook.summary.asks.iter())
        .enumerate()
        .map(|(i, (bid_level, ask_level))| {
            Row::new([
                Cell::from(i.to_string()),
                Cell::from(bid_level.price.to_string()),
                Cell::from(bid_level.amount.to_string()),
                Cell::from(bid_level.exchange.as_ref()),
                Cell::from(ask_level.price.to_string()),
                Cell::from(ask_level.amount.to_string()),
                Cell::from(ask_level.exchange.as_ref()),
            ])
        });

    Table::new(rows)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Table"))
        .widths(&[
            Constraint::Length(5),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
        ])
}

fn make_input<'a>(state: &'a LockedState) -> Paragraph<'a> {
    Paragraph::new(state.input_text)
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (type \"help\" for help, \"quit\" to quit) "),
        )
}

fn make_log_messages<'a>(state: &'a LockedState) -> List<'a> {
    let messages: Vec<ListItem> = state
        .log_messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let content = vec![Spans::from(Span::raw(format!("{}: {}", i, m)))];
            ListItem::new(content)
        })
        .collect();

    List::new(messages).block(Block::default().borders(Borders::ALL).title("Messages"))
}

fn make_status<'a>(state: &'a LockedState) -> Paragraph<'a> {
    let status = if state.is_connected {
        "Connected"
    } else {
        "Not connected"
    };

    Paragraph::new(status)
        //.style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Connection Status "),
        )
}
