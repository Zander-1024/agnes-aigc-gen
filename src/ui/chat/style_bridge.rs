use ratatui::style::{Color, Modifier, Style};

pub(crate) fn convert_core_color(color: ratatui_core::style::Color) -> Color {
    use ratatui_core::style::Color as C;
    match color {
        C::Reset => Color::Reset,
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::Gray => Color::Gray,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::White => Color::White,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
    }
}

pub(crate) fn convert_core_style(style: ratatui_core::style::Style) -> Style {
    Style {
        fg: style.fg.map(convert_core_color),
        bg: style.bg.map(convert_core_color),
        add_modifier: Modifier::from_bits_truncate(style.add_modifier.bits()),
        sub_modifier: Modifier::from_bits_truncate(style.sub_modifier.bits()),
        ..Style::default()
    }
}
