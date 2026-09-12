PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
) STRICT;

INSERT OR IGNORE INTO schema_migrations (version, applied_at)
VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

CREATE TABLE IF NOT EXISTS records (
    id TEXT PRIMARY KEY,
    content_json TEXT NOT NULL,
    origin_json TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS edges (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES records(id),
    target_id TEXT NOT NULL REFERENCES records(id),
    content_json TEXT NOT NULL,
    origin_json TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS conflicts (
    left_node_id TEXT NOT NULL REFERENCES records(id),
    right_node_id TEXT NOT NULL REFERENCES records(id),
    rule_id TEXT NOT NULL,
    content_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (left_node_id, right_node_id, rule_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    published_at TEXT,
    state TEXT NOT NULL CHECK (state IN ('candidate', 'published'))
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS snapshot_records (
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    record_id TEXT NOT NULL REFERENCES records(id),
    PRIMARY KEY (snapshot_id, record_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS snapshot_edges (
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    edge_id TEXT NOT NULL REFERENCES edges(id),
    PRIMARY KEY (snapshot_id, edge_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS snapshot_conflicts (
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    left_node_id TEXT NOT NULL,
    right_node_id TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, left_node_id, right_node_id, rule_id),
    FOREIGN KEY (left_node_id, right_node_id, rule_id)
        REFERENCES conflicts(left_node_id, right_node_id, rule_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS snapshot_sources (
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    freshness TEXT NOT NULL CHECK (freshness IN ('fresh', 'stale')),
    observed_at TEXT NOT NULL,
    cursor TEXT,
    PRIMARY KEY (snapshot_id, source_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS source_cursors (
    source_id TEXT PRIMARY KEY,
    cursor TEXT NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    updated_at TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS quarantined_inputs (
    id INTEGER PRIMARY KEY,
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    source_locator TEXT NOT NULL,
    diagnostic_code TEXT NOT NULL,
    diagnostic_message TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS current_snapshot (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id)
) STRICT;

CREATE INDEX IF NOT EXISTS snapshot_records_by_record
ON snapshot_records(record_id, snapshot_id);

CREATE INDEX IF NOT EXISTS snapshot_edges_by_edge
ON snapshot_edges(edge_id, snapshot_id);

CREATE INDEX IF NOT EXISTS quarantine_by_snapshot
ON quarantined_inputs(snapshot_id, id);

CREATE TRIGGER IF NOT EXISTS immutable_records_update
BEFORE UPDATE ON records
BEGIN
    SELECT RAISE(ABORT, 'records are immutable');
END;

CREATE TRIGGER IF NOT EXISTS immutable_records_delete
BEFORE DELETE ON records
BEGIN
    SELECT RAISE(ABORT, 'records are immutable');
END;

CREATE TRIGGER IF NOT EXISTS immutable_edges_update
BEFORE UPDATE ON edges
BEGIN
    SELECT RAISE(ABORT, 'edges are immutable');
END;

CREATE TRIGGER IF NOT EXISTS immutable_edges_delete
BEFORE DELETE ON edges
BEGIN
    SELECT RAISE(ABORT, 'edges are immutable');
END;

CREATE TRIGGER IF NOT EXISTS immutable_conflicts_update
BEFORE UPDATE ON conflicts
BEGIN
    SELECT RAISE(ABORT, 'conflicts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS immutable_conflicts_delete
BEFORE DELETE ON conflicts
BEGIN
    SELECT RAISE(ABORT, 'conflicts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS current_snapshot_requires_published_insert
BEFORE INSERT ON current_snapshot
WHEN (SELECT state FROM snapshots WHERE id = NEW.snapshot_id) <> 'published'
BEGIN
    SELECT RAISE(ABORT, 'current snapshot must be published');
END;

CREATE TRIGGER IF NOT EXISTS current_snapshot_requires_published_update
BEFORE UPDATE OF snapshot_id ON current_snapshot
WHEN (SELECT state FROM snapshots WHERE id = NEW.snapshot_id) <> 'published'
BEGIN
    SELECT RAISE(ABORT, 'current snapshot must be published');
END;
