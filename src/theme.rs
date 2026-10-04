use crate::model::ThemeColor;

/// Parses a user-supplied color into a picker theme color.
///
/// Accepts the sixteen standard terminal color names, `#RRGGBB` literals, and `ansi:N`
/// for one of the 256 palette entries.
pub fn parse_theme_color(value: &str) -> Option<ThemeColor> {
    let value = value.trim();

    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }

    if let Some(index) = value.strip_prefix("ansi:") {
        return index.parse().ok().map(ThemeColor::Ansi);
    }

    named_color(&value.to_ascii_lowercase().replace('_', "-"))
}

fn named_color(name: &str) -> Option<ThemeColor> {
    let color = match name {
        "black" => ThemeColor::Black,
        "dark-grey" | "dark-gray" => ThemeColor::DarkGrey,
        "grey" | "gray" => ThemeColor::Grey,
        "white" => ThemeColor::White,
        "red" | "light-red" => ThemeColor::Red,
        "dark-red" => ThemeColor::DarkRed,
        "green" | "light-green" => ThemeColor::Green,
        "dark-green" => ThemeColor::DarkGreen,
        "yellow" | "light-yellow" => ThemeColor::Yellow,
        "dark-yellow" => ThemeColor::DarkYellow,
        "blue" | "light-blue" => ThemeColor::Blue,
        "dark-blue" => ThemeColor::DarkBlue,
        "magenta" | "light-magenta" => ThemeColor::Magenta,
        "dark-magenta" => ThemeColor::DarkMagenta,
        "cyan" | "light-cyan" => ThemeColor::Cyan,
        "dark-cyan" => ThemeColor::DarkCyan,
        _ => return None,
    };
    Some(color)
}

fn parse_hex(hex: &str) -> Option<ThemeColor> {
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(ThemeColor::Rgb {
        r: u8::from_str_radix(&hex[0..2], 16).ok()?,
        g: u8::from_str_radix(&hex[2..4], 16).ok()?,
        b: u8::from_str_radix(&hex[4..6], 16).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_colors_case_and_separator_insensitively() {
        assert_eq!(parse_theme_color("yellow"), Some(ThemeColor::Yellow));
        assert_eq!(parse_theme_color("Dark_Grey"), Some(ThemeColor::DarkGrey));
        assert_eq!(parse_theme_color(" dark-gray "), Some(ThemeColor::DarkGrey));
    }

    #[test]
    fn parses_hex_and_ansi_palette_colors() {
        assert_eq!(
            parse_theme_color("#ffd75f"),
            Some(ThemeColor::Rgb {
                r: 255,
                g: 215,
                b: 95
            })
        );
        assert_eq!(parse_theme_color("ansi:208"), Some(ThemeColor::Ansi(208)));
    }

    #[test]
    fn rejects_unknown_and_malformed_colors() {
        assert_eq!(parse_theme_color("not-a-color"), None);
        assert_eq!(parse_theme_color("#ffd"), None);
        assert_eq!(parse_theme_color("#gggggg"), None);
        assert_eq!(parse_theme_color("ansi:300"), None);
    }
}
