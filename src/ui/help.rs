use crate::input::ToggleKey;

/// The overlay shows the keys down its right edge from this many columns on.
pub const MIN_WIDTH: u16 = 100;

/// Columns of the key list, border included.
pub const WIDTH: u16 = 24;

/// What the keys act on right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelpContext {
    Compose,
    Send,
    Timeline,
    Thread,
    ConfirmDelete,
}

/// The keys that do something in `context`, as (key, what it does), such as
/// `("d", "delete")`. The toggle key is shown as it was given. Without one,
/// as on its own, Esc's line says that it also quits.
pub fn entries(context: HelpContext, toggle: Option<ToggleKey>) -> Vec<(String, &'static str)> {
    let mut entries = match toggle {
        Some(toggle) => vec![(toggle.to_string(), "close"), ("Esc".to_owned(), "back")],
        None => vec![("Esc".to_owned(), "back / quit")],
    };
    let keys: &[(&str, &str)] = match context {
        HelpContext::Compose => &[
            ("Tab", "next focus"),
            ("Enter", "new line"),
            ("Backspace", "erase / unreply"),
        ],
        HelpContext::Send => &[("Tab", "timeline"), ("Enter", "send")],
        HelpContext::Timeline => &[
            ("Tab", "composer"),
            ("j / k", "next / previous"),
            ("h / l", "switch tab"),
            ("Enter", "open thread"),
            ("r", "reply"),
            ("q", "quote"),
            ("s", "star / unstar"),
            ("d", "delete own commit"),
        ],
        HelpContext::Thread => &[
            ("Tab", "composer"),
            ("j / k", "next / previous"),
            ("Enter", "open thread"),
            ("r", "reply"),
            ("q", "quote"),
            ("s", "star / unstar"),
            ("d", "delete own commit"),
        ],
        HelpContext::ConfirmDelete => &[("Enter", "delete")],
    };
    entries.extend(
        keys.iter()
            .map(|(key, action)| ((*key).to_owned(), *action)),
    );
    entries
}
