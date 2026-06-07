use ratatui::layout::{Constraint, Layout, Rect};

pub struct AppLayout {
    pub header: Rect,
    pub messages: Rect,
    pub sidebar: Rect,
    pub input: Rect,
    pub footer: Rect,
}

impl AppLayout {
    pub fn new(area: Rect) -> Self {
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);

        let body = Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .split(rows[1]);

        Self {
            header: rows[0],
            messages: body[0],
            sidebar: body[1],
            input: rows[2],
            footer: rows[3],
        }
    }
}

pub fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}
