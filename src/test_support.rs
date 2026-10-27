use crate::api::{Commit, CommitEntry, CommitId, Page};

pub fn id(n: u128) -> CommitId {
    uuid::Uuid::from_u128(n).into()
}

pub fn commit(n: u128) -> Commit {
    serde_json::from_value(commit_json(n)).expect("upstream-shaped fixture")
}

pub fn commit_json(n: u128) -> serde_json::Value {
    serde_json::json!({
        "id": id(n), "short_hash": format!("{n:07x}"), "body": format!("body {n}"),
        "parent_id": null, "html_url": format!("https://dev-null.sh/@someone/{}", id(n)),
        "created_at": "2026-10-20T12:00:00Z", "author": {"handle": "someone", "name": "Some One", "avatar_url": "https://example.test/avatar", "lane": 4},
        "status": "approved", "stars_count": 1, "replies_count": 0, "starred": false,
        "images": [{"url": "https://example.test/image", "width": 120, "height": 80}],
        "deleted_at": null, "body_segments": [{"type": "text", "text": "body"}],
        "quoted_commit": null, "is_saved": false
    })
}

pub fn page(ids: &[u128], next: Option<&str>) -> Page<CommitEntry> {
    Page {
        items: ids
            .iter()
            .map(|n| CommitEntry::Live(Box::new(commit(*n))))
            .collect(),
        next_cursor: next.map(|c| c.to_owned().into()),
    }
}

pub struct TestDir(pub std::path::PathBuf);

impl TestDir {
    pub fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).expect("random test directory");
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-data")
            .join(suffix);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .expect("test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
