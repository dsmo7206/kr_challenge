use super::ui_state::{UIState, RECENT_MID_PRICE_WINDOW_SECS};
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Span, Spans},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Clear, Dataset, List, ListItem, Paragraph, Row, Table,
        Tabs,
    },
    Frame,
};

pub fn draw_ui<B>(f: &mut Frame<B>, state: &mut UIState)
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

    draw_main(f, chunks[0], state);
    draw_input(f, chunks[1], state);
    draw_log_messages(f, chunks[2], state);
    draw_status(f, chunks[3], state);

    if state.show_help_popup {
        draw_help_popup(f, centered_rect(60, 40, f.size()), state);
    }

    f.set_cursor(
        // Put cursor past the end of the input text
        chunks[1].x + state.input_text.len() as u16 + 1,
        // Move one line down, from the border to the input line
        chunks[1].y + 1,
    );
}

fn draw_help_popup<B: Backend>(f: &mut Frame<B>, area: Rect, _state: &UIState) {
    let text = r"
Available commands:
  - connect <server_ipv6_address>
      Connects to the server at the given address
  - open <currency_pair> <exchange_1> [<exchange_2>, ...]
      Once connected, opens a new tab showing the aggregated orderbook
      for the given currency_pair across the given exchanges
  - show <tab_index>
      Sets the main view to the (zero-indexed) tab index
  - close <tab_index>
      Closes the tab at the (zero-indexed) tab index
  - help
      Shows this popup
  - quit (or Esc)
      Quits the program
";
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title(" Help (press any key to close) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White).bg(Color::Blue)),
    );

    f.render_widget(Clear, area); //this clears out the background
    f.render_widget(paragraph, area);
}

fn draw_main<B: Backend>(f: &mut Frame<B>, area: Rect, state: &mut UIState) {
    if state.orderbooks.is_empty() {
        draw_main_empty(f, area);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
            .split(area);

        draw_main_tabs(f, chunks[0], state);
        draw_main_prices(f, chunks[1], state);
    }
}

fn draw_main_empty<B: Backend>(f: &mut Frame<B>, area: Rect) {
    let widget = Paragraph::new(
        "Welcome to the TUI Client!

To get started, try the following inputs:

connect http://[::1]:50051
open ETH/BTC binance bitstamp",
    )
    .style(Style::default().fg(Color::Yellow));

    f.render_widget(widget, area);
}

fn draw_main_tabs<B: Backend>(f: &mut Frame<B>, area: Rect, state: &UIState) {
    let titles = state
        .orderbooks
        .iter()
        .map(|ob| &ob.title)
        .enumerate()
        .map(|(tab_index, title)| {
            if tab_index == state.selected_tab_index.unwrap() {
                Spans::from(vec![Span::styled(
                    title.as_str(),
                    Style::default().fg(Color::Yellow),
                )])
            } else {
                Spans::from(vec![Span::styled(
                    title.as_str(),
                    Style::default().fg(Color::Gray),
                )])
            }
        })
        .collect();

    let widget = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Tabs"))
        .select(state.selected_tab_index.unwrap())
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Black),
        );

    f.render_widget(widget, area);
}

fn draw_main_prices<B: Backend>(f: &mut Frame<B>, area: Rect, state: &mut UIState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(0)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(area);

    draw_main_table(f, chunks[0], state);
    draw_main_mid_chart(f, chunks[1], state);
}

fn draw_main_table<B: Backend>(f: &mut Frame<B>, area: Rect, state: &mut UIState) {
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

    let orderbook = &mut state.orderbooks[state.selected_tab_index.unwrap()];

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

    let widget = Table::new(rows)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Orderbook"))
        .widths(&[
            Constraint::Length(5),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ]);

    f.render_stateful_widget(widget, area, &mut orderbook.table_state);
}

fn draw_main_mid_chart<B: Backend>(f: &mut Frame<B>, area: Rect, state: &UIState) {
    let orderbook = &state.orderbooks[state.selected_tab_index.unwrap()];

    let recent_mid_prices = orderbook
        .recent_mid_prices
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    let max_x = recent_mid_prices.last().map_or(0.0, |(x, _)| *x);
    let min_x = max_x - RECENT_MID_PRICE_WINDOW_SECS as f64;

    let mut min_y = orderbook.min_price_seen.unwrap_or(0.0);
    let mut max_y = orderbook.max_price_seen.unwrap_or(1.0);

    let diff_y = max_y - min_y;
    min_y -= diff_y * 0.2;
    max_y += diff_y * 0.2;

    let datasets = vec![Dataset::default()
        .name("Mid price")
        .marker(symbols::Marker::Braille)
        .style(Style::default().fg(Color::Cyan))
        .data(&recent_mid_prices)];
    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(Span::styled(
                    "Mid price",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .title("Time (since connection open)")
                .style(Style::default().fg(Color::Gray))
                .bounds([min_x, max_x])
                .labels(vec![
                    Span::styled(
                        format!("{:.1}", min_x),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:.1}", max_x),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("Price")
                .style(Style::default().fg(Color::Gray))
                .bounds([min_y, max_y])
                .labels(vec![
                    Span::styled(
                        format!("{:.5}", min_y),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("{:.5}", 0.5 * (min_y + max_y))),
                    Span::styled(
                        format!("{:.5}", max_y),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
        );
    f.render_widget(chart, area);
}

fn draw_input<B: Backend>(f: &mut Frame<B>, area: Rect, state: &UIState) {
    let widget = Paragraph::new(state.input_text.as_ref())
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (type \"help\" for help, \"quit\" to quit) "),
        );

    f.render_widget(widget, area);
}

fn draw_log_messages<B: Backend>(f: &mut Frame<B>, area: Rect, state: &mut UIState) {
    let messages: Vec<ListItem> = state
        .log_messages
        .iter()
        .map(|(index, msg)| {
            let content = vec![Spans::from(Span::raw(format!("{index}: {msg}")))];
            ListItem::new(content)
        })
        .collect();

    let widget =
        List::new(messages).block(Block::default().borders(Borders::ALL).title("Messages"));

    f.render_stateful_widget(widget, area, &mut state.log_messages_list_state);
}

fn draw_status<B: Backend>(f: &mut Frame<B>, area: Rect, state: &UIState) {
    let status = if state.is_connected {
        "Connected"
    } else {
        "Not connected"
    };

    let widget = Paragraph::new(status).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Connection Status "),
    );

    f.render_widget(widget, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}
