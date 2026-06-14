use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub const MENU_ITEMS: [&str; 7] =
    ["Generate Image", "Generate Video", "Video Tasks", "Assets", "Settings", "Chat", "Quit"];

pub struct HomeMenu {
    pub list_state: ListState,
}

impl HomeMenu {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self { list_state }
    }

    pub fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    pub fn select_next(&mut self) {
        let next = (self.selected() + 1) % MENU_ITEMS.len();
        self.list_state.select(Some(next));
    }

    pub fn select_previous(&mut self) {
        let prev = if self.selected() == 0 {
            MENU_ITEMS.len() - 1
        } else {
            self.selected() - 1
        };
        self.list_state.select(Some(prev));
    }

    pub fn select_by_key(&mut self, key: char) -> bool {
        let index = match key {
            '1' => Some(0),
            '2' => Some(1),
            '3' => Some(2),
            '4' => Some(3),
            '5' => Some(4),
            '6' => Some(5),
            '7' | 'q' => Some(6),
            _ => None,
        };
        if let Some(i) = index {
            self.list_state.select(Some(i));
            true
        } else {
            false
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, running_count: usize) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(area);

        let items: Vec<ListItem> = MENU_ITEMS
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let prefix = format!("{}. ", i + 1);
                let text = if i == 2 && running_count > 0 {
                    format!("{label} ({running_count} running)")
                } else {
                    (*label).to_string()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::Cyan)),
                    Span::raw(text),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Agnes Dashboard — Home").borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        frame.render_stateful_widget(list, chunks[0], &mut self.list_state);
        frame.render_widget(
            Paragraph::new("↑↓/Enter navigate  1-7 shortcuts  q quit on Quit")
                .style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );
    }
}
