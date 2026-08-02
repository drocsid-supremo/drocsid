use ratatui::style::{Color, Modifier, Style};

pub const PASTEL_YELLOW: Color = Color::Rgb(245, 229, 168);
pub const PASTEL_YELLOW_BORDER: Color = Color::Rgb(226, 208, 140);

pub fn brand_style() -> Style {
    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn client_badge_style() -> Style {
    Style::new().fg(Color::Black).bg(Color::Green)
}

pub fn messages_border_style() -> Style {
    Style::new().fg(Color::Blue)
}

pub fn session_border_style() -> Style {
    Style::new().fg(Color::Green)
}

pub fn input_border_style() -> Style {
    Style::new().fg(PASTEL_YELLOW_BORDER)
}

pub fn mention_selected_style() -> Style {
    Style::new().fg(Color::Black).bg(PASTEL_YELLOW)
}

pub fn mention_plain_style() -> Style {
    Style::new().fg(Color::White)
}

pub fn system_message_style() -> Style {
    Style::new().fg(Color::Magenta)
}

pub fn username_style(pending: bool) -> Style {
    if pending {
        Style::new().fg(Color::Gray)
    } else {
        Style::new().fg(Color::Cyan)
    }
}

pub fn timestamp_style() -> Style {
    Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)
}

pub fn body_style(pending: bool) -> Style {
    if pending {
        Style::new().fg(Color::Gray)
    } else {
        Style::new().fg(Color::White)
    }
}

pub fn fallback_message_style(pending: bool) -> Style {
    if pending {
        Style::new().fg(Color::Gray)
    } else {
        Style::new()
    }
}

pub fn disconnected_title_style() -> Style {
    Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn disconnected_border_style() -> Style {
    Style::new().fg(Color::Red)
}

pub fn muted_style() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn users_title_style() -> Style {
    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
}

pub fn online_user_style() -> Style {
    Style::new().fg(Color::Cyan)
}

pub fn input_text_style() -> Style {
    Style::new().fg(Color::White)
}
