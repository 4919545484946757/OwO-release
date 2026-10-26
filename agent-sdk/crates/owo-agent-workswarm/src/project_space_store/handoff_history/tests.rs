use super::*;

struct DatabaseFixture(std::path::PathBuf);
impl DatabaseFixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("owo-handoff-history-{}.db", uuid::Uuid::new_v4()));
        // Reserve a unique fixture; never open an existing unrelated database.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for DatabaseFixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.as_os_str().to_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(path));
        }
    }
}

#[tokio::test]
async fn reverse_handoffs_cover_tied_timestamps_without_cross_project_leaks() {
    let store =
        SqliteProjectSpaceStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
    {
        let mut conn = store.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..10_000 {
            let id = format!("h-{i:05}");
            let project = if i % 2 == 0 { "one" } else { "two" };
            let record = serde_json::json!({"handoff_id":id,"from_member":"writer",
                "to_member":"reader","completed_summary":id,
                "output_artifact_refs":["actual-asset"],"created_at":"2026-10-25T00:00:00Z"});
            tx.execute(
                "INSERT INTO handoffs VALUES(?1,?2,?3,?4)",
                params![id, project, record.to_string(), "2026-10-25T00:00:00Z"],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let mut before = None;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = store
            .handoffs_before("one", before.as_deref(), 127)
            .unwrap();
        assert!(page.handoffs.len() <= 127);
        assert_eq!(
            page.next_before,
            page.handoffs.first().map(|h| h.handoff_id.clone())
        );
        for h in &page.handoffs {
            assert_eq!(h.output_artifact_refs, vec!["actual-asset"]);
            assert!(seen.insert(h.handoff_id.clone()), "duplicate across pages");
            let n: usize = h.handoff_id[2..].parse().unwrap();
            assert_eq!(n % 2, 0);
        }
        assert!(page
            .handoffs
            .windows(2)
            .all(|w| w[0].handoff_id < w[1].handoff_id));
        if !page.has_more {
            break;
        }
        before = page.next_before;
    }
    assert_eq!(seen.len(), 5000);
    let empty = store.handoffs_before("one", Some("h-00000"), 128).unwrap();
    assert!(empty.handoffs.is_empty());
    assert!(empty.next_before.is_none());
    assert!(!empty.has_more);
    for cursor in ["h-00001", "missing", ""] {
        assert!(store.handoffs_before("one", Some(cursor), 64).is_err());
    }
    assert!(store.handoffs_before("one", None, 0).is_err());
    assert!(store.handoffs_before("one", None, 129).is_err());
    assert!(store
        .handoffs_before("empty", None, 64)
        .unwrap()
        .handoffs
        .is_empty());
}

#[tokio::test]
async fn reverse_handoff_cursor_uses_persisted_order_after_update_and_reopen() {
    let fixture = DatabaseFixture::new();
    let path = fixture.0.clone();
    let store = SqliteProjectSpaceStore::open(&path).unwrap();
    let record = |id: &str, time: &str| -> HandoffRecord {
        serde_json::from_value(serde_json::json!({"handoff_id":id,"from_member":"writer",
            "to_member":"reader","completed_summary":id,"created_at":time}))
        .unwrap()
    };
    store
        .save_handoff(&record("a", "2026-01-01"), "p")
        .await
        .unwrap();
    store
        .save_handoff(&record("b", "2026-02-01"), "p")
        .await
        .unwrap();
    let first = store.handoffs_before("p", None, 1).unwrap();
    assert_eq!(first.next_before.as_deref(), Some("b"));
    store
        .save_handoff(&record("c", "2026-03-01"), "p")
        .await
        .unwrap();
    // Updating the JSON does not mutate the durable order columns.
    store
        .save_handoff(&record("b", "2099-01-01"), "p")
        .await
        .unwrap();
    drop(store);
    let store = SqliteProjectSpaceStore::open(&path).unwrap();
    let older = store
        .handoffs_before("p", first.next_before.as_deref(), 1)
        .unwrap();
    assert_eq!(older.handoffs[0].handoff_id, "a");
    assert!(!older.has_more);
    assert_eq!(
        store.handoffs_before("p", None, 1).unwrap().handoffs[0].handoff_id,
        "c"
    );
}
