use crate::api::Lane;
use ratatui::style::{Color, Modifier, Style};

pub(super) fn accent() -> Style {
    Style::default().fg(Color::Cyan)
}

/// /dev/null's own lane colors, from its dark theme, which reads better on
/// the dark backgrounds most terminals have.
const LANES: [(u8, u8, u8); 6] = [
    (0xff, 0x8a, 0x4c),
    (0x51, 0xcf, 0x66),
    (0xda, 0x77, 0xf2),
    (0x3b, 0xc9, 0xdb),
    (0xf0, 0x65, 0x95),
    (0xfc, 0xc4, 0x19),
];

/// A username, in its lane color, or the accent when it has none, a picked
/// one that is not `#rrggbb`, or a number past /dev/null's lanes.
pub(super) fn user(lane: Option<&Lane>) -> Style {
    let rgb = match lane {
        Some(Lane::Picked(lane)) => lane
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .map(|rgb| ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)),
        Some(Lane::Automatic(lane)) => LANES.get(usize::from(*lane)).copied(),
        None => None,
    };
    match rgb {
        Some((r, g, b)) => Style::default()
            .fg(Color::Rgb(r, g, b))
            .add_modifier(Modifier::BOLD),
        None => accent(),
    }
}

pub(super) fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub(super) fn quote() -> Style {
    Style::default().fg(Color::Gray)
}

pub(super) fn selected_header() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub(super) fn selection_bar(focused: bool) -> Style {
    if focused { accent() } else { muted() }
}

pub(super) fn stars(starred: bool) -> Style {
    if starred {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    }
}

pub(super) fn active_tab() -> Style {
    accent().add_modifier(Modifier::BOLD)
}

/// Filled, unlike the list's selection: reversed accent, so the label takes
/// the terminal's own background color and stays readable on any theme.
pub(super) fn focused_button() -> Style {
    accent().add_modifier(Modifier::BOLD | Modifier::REVERSED)
}

pub(super) fn background() -> Style {
    Style::default().fg(Color::DarkGray)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Picked `#rrggbb` lanes and numbered lanes color a username; anything else falls back to the accent.
    #[test]
    fn user_lane_colors() {
        let rgb = |r, g, b| {
            Style::default()
                .fg(Color::Rgb(r, g, b))
                .add_modifier(Modifier::BOLD)
        };
        assert_eq!(
            user(Some(&Lane::Picked("#1A2b3c".into()))),
            rgb(0x1a, 0x2b, 0x3c)
        );
        assert_eq!(user(Some(&Lane::Automatic(0))), rgb(0xff, 0x8a, 0x4c));
        assert_eq!(user(Some(&Lane::Automatic(5))), rgb(0xfc, 0xc4, 0x19));
        assert_eq!(user(Some(&Lane::Automatic(6))), accent());
        assert_eq!(user(None), accent());
        for lane in ["", "1a2b3c", "#1a2b3", "#+1a2b3", "#1a2b3g"] {
            assert_eq!(user(Some(&Lane::Picked(lane.into()))), accent());
        }
    }
}
