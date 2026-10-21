use crate::api::CommitEntry;
use crate::ui::theme;
use ratatui::text::{Line, Span};

pub(super) fn lines(
    entry: &CommitEntry,
    width: u16,
    full: bool,
    now: jiff::Timestamp,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match entry {
        CommitEntry::Live(commit) => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("@{}", commit.author.username),
                    theme::user(commit.author.lane.as_ref()),
                ),
                Span::styled(
                    format!(
                        "  {}  {}",
                        crate::ui::format::relative_time(commit.created_at, now),
                        commit.short_hash
                    ),
                    theme::accent(),
                ),
            ]));
            let (body, omitted) = crate::ui::format::fold(
                &commit.body,
                width,
                if full {
                    usize::MAX
                } else {
                    crate::ui::format::FOLDED_LINES
                },
            );
            lines.extend(body.into_iter().map(Line::raw));
            if omitted > 0 {
                lines.push(Line::styled(
                    format!("… (+{omitted} lines)"),
                    theme::muted(),
                ));
            }
            if let Some(quote) = &commit.quoted_commit {
                let text = match quote {
                    CommitEntry::Live(c) => {
                        format!("Quote @{} {}: {}", c.author.username, c.short_hash, c.body)
                    }
                    CommitEntry::Deleted(_) => "Quote: [deleted]".to_owned(),
                };
                lines.extend(
                    crate::ui::format::fold(&text, width, if full { usize::MAX } else { 2 })
                        .0
                        .into_iter()
                        .map(|line| Line::styled(line, theme::quote())),
                );
            }
            for image in &commit.images {
                lines.extend(
                    crate::ui::format::fold(&format!("[image] {}", image.url), width, usize::MAX)
                        .0
                        .into_iter()
                        .map(Line::raw),
                );
            }
            let starred = commit.starred == Some(true);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "{}{} stars",
                        if starred { "* " } else { "" },
                        commit.stars_count.unwrap_or(0)
                    ),
                    theme::stars(starred),
                ),
                Span::raw(format!("  {} replies", commit.replies_count.unwrap_or(0))),
            ]));
        }
        CommitEntry::Deleted(commit) => lines.push(Line::raw(format!(
            "{} [deleted]",
            crate::api::ShortHash::from(commit.id)
        ))),
    }
    lines.push(Line::raw(""));

    lines
}
