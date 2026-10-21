use crate::api::{NotificationGroup, NotificationKind};
use crate::ui::{format, notifications, theme};
use ratatui::text::{Line, Span};

pub(super) fn lines(
    group: &NotificationGroup,
    width: u16,
    new: bool,
    now: jiff::Timestamp,
) -> Vec<Line<'static>> {
    let mut header = Vec::new();
    if let Some(actors) = notifications::actors(group) {
        header.push(Span::styled(actors, theme::user(group.actor.lane.as_ref())));
        header.push(Span::raw("  "));
    }
    header.push(Span::styled(
        format!(
            "{}{}",
            format::relative_time(group.created_at, now),
            if new { "  new" } else { "" }
        ),
        theme::accent(),
    ));
    let mut lines = vec![Line::from(header)];
    lines.extend(
        format::fold(notifications::wording(group), width, usize::MAX)
            .0
            .into_iter()
            .map(Line::raw),
    );
    if group.kind == NotificationKind::Rejected
        && let Some(reason) = &group.reason
    {
        lines.extend(
            format::fold(&format!("Reason: {reason}"), width, usize::MAX)
                .0
                .into_iter()
                .map(Line::raw),
        );
    }
    if let Some(commit) = &group.commit {
        lines.extend(
            format::fold(&commit.body, width, 3)
                .0
                .into_iter()
                .map(Line::raw),
        );
    }
    lines.push(Line::raw(""));
    lines
}
