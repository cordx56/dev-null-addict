use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::ui::{App, format, theme};

impl App {
    pub(in crate::ui) fn post_area(&self, area: Rect) -> Rect {
        if !self.showing_profile() {
            return area;
        }
        let height = lines(self, area.width)
            .len()
            .min(usize::from(area.height / 2)) as u16;
        Rect::new(area.x, area.y + height, area.width, area.height - height)
    }
}

pub(super) fn lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let Some(profile) = app
        .profiles
        .shown()
        .filter(|view| view.unavailable.is_none())
        .and_then(|view| view.profile.as_ref())
    else {
        return Vec::new();
    };
    let width = width.saturating_sub(1);
    let mut identity = Vec::new();
    if let Some(name) = &profile.user.name {
        identity.push(Span::styled(
            name.clone(),
            theme::user(profile.user.lane.as_ref()),
        ));
        identity.push(Span::raw(" "));
    }
    identity.push(Span::raw(format!("@{}", profile.user.username)));
    if app.profiles.other.is_some() && app.me.as_ref() != Some(&profile.user.username) {
        if profile.you_follow {
            identity.push(Span::raw("  following"));
        }
        if profile.follows_you {
            identity.push(Span::raw("  follows you"));
        }
    }
    let mut lines = wrap(identity, width);
    if let Some(bio) = &profile.bio {
        lines.extend(format::fold(bio, width, 3).0.into_iter().map(Line::raw));
    }
    let details = [&profile.company, &profile.location, &profile.website]
        .into_iter()
        .filter_map(|value| value.as_deref())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !details.is_empty() {
        lines.extend(
            format::fold(&details, width, usize::MAX)
                .0
                .into_iter()
                .map(Line::raw),
        );
    }
    let counts = [
        (profile.commits_count, "commits"),
        (profile.followers_count, "followers"),
        (profile.following_count, "following"),
    ]
    .into_iter()
    .filter_map(|(count, label)| count.map(|count| format!("{count} {label}")))
    .chain(std::iter::once(format!(
        "joined {}",
        profile.created_at.to_zoned(jiff::tz::TimeZone::UTC).date()
    )))
    .collect::<Vec<_>>()
    .join("  ");
    lines.extend(
        format::fold(&counts, width, usize::MAX)
            .0
            .into_iter()
            .map(Line::raw),
    );
    lines.push(Line::raw(""));
    indent(lines)
}

pub(super) fn message_lines(message: &str, width: u16) -> Vec<Line<'static>> {
    indent(
        format::fold(message, width.saturating_sub(1), usize::MAX)
            .0
            .into_iter()
            .map(Line::raw)
            .collect(),
    )
}

fn indent(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(" "));
            line
        })
        .collect()
}

fn wrap(spans: Vec<Span<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];
    let mut columns = 0;
    for span in spans {
        for ch in span.content.chars() {
            let size = ch.width().unwrap_or(0);
            if ch == '\n' || (columns + size > usize::from(width.max(1)) && columns > 0) {
                lines.push(Line::default());
                columns = 0;
            }
            if ch != '\n' {
                lines
                    .last_mut()
                    .expect("at least one line")
                    .spans
                    .push(Span::styled(ch.to_string(), span.style));
                columns += size;
            }
        }
    }
    lines
}
