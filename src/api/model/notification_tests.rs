use super::*;
use crate::test_support::commit_json;

// Notification resources omit unloaded commit relations and counts without invalidating the page.
#[test]
fn notification_resources_decode_sparse_commits_and_future_kinds() {
    let mut commit = commit_json(1);
    for field in [
        "stars_count",
        "replies_count",
        "starred",
        "images",
        "quoted_commit",
    ] {
        commit.as_object_mut().unwrap().remove(field);
    }
    let kinds = [
        ("followed", NotificationKind::Followed),
        ("replied", NotificationKind::Replied),
        ("mentioned", NotificationKind::Mentioned),
        ("quoted", NotificationKind::Quoted),
        ("starred", NotificationKind::Starred),
        ("approved", NotificationKind::Approved),
        ("rejected", NotificationKind::Rejected),
        ("leveled_up", NotificationKind::LeveledUp),
        ("future_event", NotificationKind::Other),
    ];
    let groups: Vec<_> = kinds.iter().enumerate().map(|(i, (kind, _))| {
        serde_json::json!({
            "id": uuid::Uuid::from_u128(i as u128 + 1), "type": kind,
            "subject_id": null, "reason": if *kind == "rejected" { Some("spam") } else { None },
            "read_at": "2026-10-22T12:00:00Z", "created_at": "2026-10-22T11:00:00Z",
            "actor": { "username": "someone", "name": "Some One", "is_followed": true },
            "group_id": uuid::Uuid::from_u128(i as u128 + 1),
            "event_count": 4, "actors_count": 2, "unread_count": 0,
            "commit": if ["followed", "leveled_up"].contains(kind) { serde_json::Value::Null } else { commit.clone() },
        })
    }).collect();
    let mut response = serde_json::json!({
        "data": groups, "meta": {"next_cursor": "opaque+/="},
        "read_at": "2026-10-22T12:00:00Z", "filters": {"type": "all"}
    });
    response["data"][1]["actors"] = serde_json::json!([
        {"username": "someone", "name": "Some One"}, {"username": "another", "name": null}
    ]);
    let page: NotificationPage = serde_json::from_value(response).unwrap();
    assert_eq!(
        page.page.next_cursor.as_ref().unwrap().as_str(),
        "opaque+/="
    );
    assert_eq!(page.page.items.len(), kinds.len());
    for (group, (_, expected)) in page.page.items.iter().zip(kinds) {
        assert_eq!(group.kind, expected);
    }
    assert!(page.page.items[0].commit.is_none());
    assert!(page.page.items[0].actors.is_empty());
    assert_eq!(page.page.items[1].actors.len(), 2);
    let commit = page.page.items[1].commit.as_ref().unwrap();
    assert!(commit.images.is_empty());
    assert_eq!(
        (commit.stars_count, commit.replies_count, commit.starred),
        (None, None, None)
    );
    assert!(commit.quoted_commit.is_none());
    assert_eq!(page.page.items[6].reason.as_deref(), Some("spam"));
    let roundtrip: NotificationPage =
        serde_json::from_value(serde_json::to_value(&page).unwrap()).unwrap();
    assert_eq!(page, roundtrip);
}

// Freshness is an inclusive instant comparison, independent of unread_count and timestamp offsets.
#[test]
fn fresh_uses_the_read_watermark() {
    let page: NotificationPage = serde_json::from_value(serde_json::json!({
        "data": ([None, Some("2026-10-22T11:59:59Z"), Some("2026-10-22T21:00:00+09:00"), Some("2026-10-22T12:00:01Z")].iter().enumerate().map(|(i, at)| serde_json::json!({
            "id": uuid::Uuid::from_u128(i as u128 + 1), "type": "followed",
            "read_at": at, "created_at": "2026-10-22T11:00:00Z",
            "actor": {"username": "someone"}, "actors_count": 1, "event_count": 1, "unread_count": 0,
        })).collect::<Vec<_>>()),
        "meta": {"next_cursor": null}, "read_at": "2026-10-22T12:00:00Z"
    })).unwrap();
    assert_eq!(
        page.fresh().map(|group| group.id).collect::<Vec<_>>(),
        [3, 4].map(|n| NotificationId::from(uuid::Uuid::from_u128(n)))
    );
}

// The unread count belongs to the envelope, while identity belongs to data.
#[test]
fn current_user_reads_top_level_unread_count() {
    let current: CurrentUser = serde_json::from_value(serde_json::json!({
        "data": {"username": "someone", "name": "Some One", "lane": 4, "unread_notifications_count": 99},
        "unread_notifications_count": 7,
    })).unwrap();
    assert_eq!(current.user.username.as_str(), "someone");
    assert_eq!(current.unread_notifications, 7);
}
