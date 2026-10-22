use ratatui::layout::{Constraint, Layout, Rect};

use super::{App, help};

/// How the overlay occupies the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverlayLayout {
    /// A centered window over the shell, falling back to full screen on small terminals.
    #[default]
    Float,
    FullScreen,
}

pub(super) struct Window {
    pub outer: Rect,
    pub inner: Rect,
    pub floating: bool,
}

pub(super) fn window(layout: OverlayLayout, area: Rect) -> Window {
    let width = (u32::from(area.width) * 4 / 5) as u16;
    let height = (u32::from(area.height) * 4 / 5) as u16;
    if layout == OverlayLayout::FullScreen || width < 60 || height < 16 {
        return Window {
            outer: area,
            inner: area,
            floating: false,
        };
    }
    let outer = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    Window {
        outer,
        inner: outer.inner(ratatui::layout::Margin::new(1, 1)),
        floating: true,
    }
}

pub(super) struct Areas {
    pub window: Window,
    pub target: Rect,
    pub composer: Rect,
    pub send: Rect,
    pub tabs: Rect,
    pub list: Rect,
    pub help: Rect,
}

pub(super) fn areas(app: &App, area: Rect) -> Areas {
    let window = window(app.layout, area);
    let area = window.inner;
    let parts = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(if window.outer.width >= help::MIN_WIDTH {
            help::WIDTH
        } else {
            0
        }),
    ])
    .split(area);
    let target_height = u16::from(app.compose.target().is_some());
    let draft_height = app
        .compose
        .height(parts[0].width.saturating_sub(2), (area.height / 2).max(1));
    let rows = Layout::vertical([
        Constraint::Length(target_height),
        Constraint::Length(draft_height + 2),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .split(parts[0]);

    Areas {
        window,
        target: rows[0],
        composer: rows[1],
        send: rows[2],
        tabs: rows[3],
        list: rows[4],
        help: parts[1],
    }
}
