use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use rationale_model::{Conflict, KernelRequest, RecordStatus, SourceKind};

use super::{
    CandidateMetadata, CandidateSnapshot, EvidenceStore, Freshness, QuarantineDiagnostic,
    SliceLimits, SourceState, StoreError,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDatabase {
    path: PathBuf,
}

impl TempDatabase {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "rationale-store-{name}-{}-{sequence}.sqlite",
            std::process::id()
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for path in [
            self.path.clone(),
            PathBuf::from(format!("{}-wal", self.path.display())),
            PathBuf::from(format!("{}-shm", self.path.display())),
        ] {
            let _ = fs::remove_file(path);
        }
    }
}

fn request() -> KernelRequest {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/protocol/request-established.json");
    let source = fs::read_to_string(path).expect("fixture should be readable");
    serde_json::from_str(&source).expect("fixture should match protocol types")
}

fn metadata(id: &str) -> CandidateMetadata {
    CandidateMetadata {
        id: id.to_owned(),
        created_at: "2026-09-12T10:00:00Z".to_owned(),
    }
}

fn source(cursor: &str) -> SourceState {
    SourceState {
        source_id: "local-git".to_owned(),
        source_kind: SourceKind::Git,
        freshness: Freshness::Fresh,
        observed_at: "2026-09-12T10:00:00Z".to_owned(),
        cursor: Some(cursor.to_owned()),
    }
}

fn limits() -> SliceLimits {
    SliceLimits {
        max_nodes: 64,
        max_edges: 64,
        max_conflicts: 64,
    }
}

fn insert_request(
    candidate: &CandidateSnapshot<'_>,
    request: &KernelRequest,
) -> Result<(), StoreError> {
    for node in &request.nodes {
        candidate.insert_record(node)?;
    }
    for edge in &request.edges {
        candidate.insert_edge(edge)?;
    }
    for conflict in &request.conflicts {
        candidate.insert_conflict(conflict)?;
    }
    Ok(())
}

fn publish_fixture(store: &mut EvidenceStore, id: &str, cursor: &str) {
    let request = request();
    let candidate = store
        .begin_candidate(metadata(id))
        .expect("candidate should begin");
    insert_request(&candidate, &request).expect("fixture should insert");
    candidate
        .set_source_state(&source(cursor))
        .expect("source state should insert");
    candidate
        .publish("2026-09-12T10:01:00Z")
        .expect("candidate should publish");
}

#[test]
fn published_snapshot_round_trips_evidence_and_metadata() {
    let mut store = EvidenceStore::open_in_memory().expect("store should open");
    let request = request();
    let candidate = store
        .begin_candidate(metadata("snapshot-1"))
        .expect("candidate should begin");
    insert_request(&candidate, &request).expect("fixture should insert");
    let conflict = Conflict {
        left_node_id: "decision-1".to_owned(),
        right_node_id: "verification-1".to_owned(),
        rule_id: "conflict.fixture".to_owned(),
    };
    candidate
        .insert_conflict(&conflict)
        .expect("conflict should insert");
    candidate
        .set_source_state(&source("cursor-1"))
        .expect("source state should insert");
    candidate
        .quarantine(&QuarantineDiagnostic {
            source_locator: "docs/invalid.md".to_owned(),
            code: "invalid_front_matter".to_owned(),
            message: "rationale metadata is not an object".to_owned(),
        })
        .expect("diagnostic should insert");
    candidate
        .publish("2026-09-12T10:01:00Z")
        .expect("candidate should publish");

    let slice = store
        .load_current_slice(limits())
        .expect("slice should load")
        .expect("snapshot should exist");
    assert_eq!(slice.snapshot_id, "snapshot-1");
    assert_eq!(slice.nodes.len(), request.nodes.len());
    assert_eq!(slice.edges.len(), request.edges.len());
    assert_eq!(slice.conflicts, vec![conflict]);
    assert_eq!(slice.sources, vec![source("cursor-1")]);
    assert_eq!(
        store.current_freshness().expect("freshness should load"),
        vec![source("cursor-1")]
    );

    let expected_origin = request
        .nodes
        .iter()
        .find(|node| node.id == "code-1")
        .expect("fixture should contain code target")
        .origin
        .clone();
    assert_eq!(
        store.record_origin("code-1").expect("origin should load"),
        Some(expected_origin)
    );
    assert_eq!(
        store.current_quarantine().expect("diagnostic should load"),
        vec![QuarantineDiagnostic {
            source_locator: "docs/invalid.md".to_owned(),
            code: "invalid_front_matter".to_owned(),
            message: "rationale metadata is not an object".to_owned(),
        }]
    );
}

#[test]
fn immutable_entities_are_idempotent_and_reject_content_mismatch() {
    let mut store = EvidenceStore::open_in_memory().expect("store should open");
    publish_fixture(&mut store, "snapshot-1", "cursor-1");
    let request = request();

    let candidate = store
        .begin_candidate(metadata("snapshot-2"))
        .expect("candidate should begin");
    insert_request(&candidate, &request).expect("same content should be idempotent");
    insert_request(&candidate, &request).expect("duplicate membership should be idempotent");
    candidate
        .publish("2026-09-12T10:02:00Z")
        .expect("idempotent candidate should publish");

    let record_count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
        .expect("record count should load");
    assert_eq!(
        usize::try_from(record_count).expect("record count should be non-negative"),
        request.nodes.len()
    );

    let mut changed = request.nodes[0].clone();
    changed.status = RecordStatus::Historical;
    let candidate = store
        .begin_candidate(metadata("snapshot-3"))
        .expect("candidate should begin");
    let error = candidate
        .insert_record(&changed)
        .expect_err("same ID with different bytes should fail");
    assert!(matches!(
        error,
        StoreError::ContentMismatch {
            kind: "records",
            ..
        }
    ));
    candidate.abandon().expect("candidate should roll back");
}

#[test]
fn readers_keep_previous_snapshot_while_candidate_is_abandoned() {
    let database = TempDatabase::new("abandon");
    let mut writer = EvidenceStore::open(database.path()).expect("writer should open");
    let reader = EvidenceStore::open(database.path()).expect("reader should open");
    publish_fixture(&mut writer, "snapshot-1", "cursor-1");

    let candidate = writer
        .begin_candidate(metadata("snapshot-2"))
        .expect("candidate should begin");
    candidate
        .insert_record(&request().nodes[0])
        .expect("partial candidate should accept a record");
    assert_eq!(
        reader
            .current_snapshot_id()
            .expect("reader should remain available")
            .as_deref(),
        Some("snapshot-1")
    );
    candidate.abandon().expect("candidate should roll back");
    assert_eq!(
        reader
            .current_snapshot_id()
            .expect("reader should remain available")
            .as_deref(),
        Some("snapshot-1")
    );
}

#[test]
fn publication_failure_rolls_back_pointer_and_cursor() {
    let database = TempDatabase::new("publication-failure");
    let mut writer = EvidenceStore::open(database.path()).expect("writer should open");
    let reader = EvidenceStore::open(database.path()).expect("reader should open");
    publish_fixture(&mut writer, "snapshot-1", "cursor-1");
    writer
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER inject_publication_failure
             BEFORE UPDATE ON current_snapshot
             BEGIN
                 SELECT RAISE(ABORT, 'injected publication failure');
             END;",
        )
        .expect("failure trigger should install");

    let request = request();
    let candidate = writer
        .begin_candidate(metadata("snapshot-2"))
        .expect("candidate should begin");
    insert_request(&candidate, &request).expect("fixture should insert");
    candidate
        .set_source_state(&source("cursor-2"))
        .expect("source state should insert");
    assert!(candidate.publish("2026-09-12T10:02:00Z").is_err());

    assert_eq!(
        reader
            .current_snapshot_id()
            .expect("pointer should load")
            .as_deref(),
        Some("snapshot-1")
    );
    assert_eq!(
        reader
            .source_cursor("local-git")
            .expect("published cursor should remain")
            .as_deref(),
        Some("cursor-1")
    );
}

#[test]
fn candidate_cannot_publish_relationships_outside_membership() {
    let mut store = EvidenceStore::open_in_memory().expect("store should open");
    publish_fixture(&mut store, "snapshot-1", "cursor-1");
    let request = request();
    let candidate = store
        .begin_candidate(metadata("snapshot-2"))
        .expect("candidate should begin");
    candidate
        .insert_edge(&request.edges[0])
        .expect("immutable edge should already exist");
    let error = candidate
        .publish("2026-09-12T10:02:00Z")
        .expect_err("edge endpoints must belong to the candidate");
    assert!(matches!(error, StoreError::UnresolvedReference { .. }));
}

#[test]
fn stable_slice_enforces_explicit_bounds() {
    let mut store = EvidenceStore::open_in_memory().expect("store should open");
    publish_fixture(&mut store, "snapshot-1", "cursor-1");
    let error = store
        .load_current_slice(SliceLimits {
            max_nodes: 1,
            max_edges: 64,
            max_conflicts: 64,
        })
        .expect_err("oversized slice should fail closed");
    assert!(matches!(
        error,
        StoreError::SliceLimitExceeded { kind: "nodes", .. }
    ));
}

#[test]
fn generated_database_is_inspectable_with_sqlite_cli() {
    let database = TempDatabase::new("cli");
    {
        let mut store = EvidenceStore::open(database.path()).expect("store should open");
        publish_fixture(&mut store, "snapshot-1", "cursor-1");
    }

    let output = Command::new("sqlite3")
        .arg(database.path())
        .arg("PRAGMA integrity_check; SELECT snapshot_id FROM current_snapshot;")
        .output()
        .expect("sqlite3 CLI must be installed for the inspectability smoke test");
    assert!(
        output.status.success(),
        "sqlite3 should open generated database"
    );
    let stdout = String::from_utf8(output.stdout).expect("sqlite3 output should be UTF-8");
    assert_eq!(stdout, "ok\nsnapshot-1\n");
}

#[test]
fn corrupted_database_is_rejected_without_replacement() {
    let database = TempDatabase::new("corrupt");
    fs::write(database.path(), b"not a SQLite database")
        .expect("corrupt fixture should be written");
    let before = fs::read(database.path()).expect("fixture should be readable");
    assert!(EvidenceStore::open(database.path()).is_err());
    assert_eq!(
        fs::read(database.path()).expect("failed open must retain the source"),
        before
    );
}

#[test]
fn incompatible_existing_schema_fails_migration_closed() {
    let database = TempDatabase::new("migration");
    {
        let connection =
            rusqlite::Connection::open(database.path()).expect("fixture database should open");
        connection
            .execute_batch("CREATE TABLE records (unexpected TEXT NOT NULL);")
            .expect("incompatible fixture schema should be created");
    }
    assert!(EvidenceStore::open(database.path()).is_err());
    let connection = rusqlite::Connection::open(database.path())
        .expect("failed migration should not destroy the database");
    let columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('records') WHERE name = 'unexpected'",
            [],
            |row| row.get(0),
        )
        .expect("fixture schema should remain inspectable");
    assert_eq!(columns, 1);
    let migration_table: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .expect("schema should remain queryable");
    assert_eq!(migration_table, 0, "failed DDL must roll back atomically");
}
