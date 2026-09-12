//! Transactional SQLite storage for immutable Rationale evidence snapshots.

use std::path::Path;

use rationale_model::{Conflict, EvidenceEdge, EvidenceNode, Origin, SourceKind};
use rationale_protocol::{CanonicalJsonError, to_canonical_json};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use thiserror::Error;

const INITIAL_MIGRATION: &str = include_str!("../../../migrations/0001_initial.sql");

/// Failure while building, publishing, or reading an evidence snapshot.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite rejected an operation.
    #[error("evidence database failed: {0}")]
    Database(#[from] rusqlite::Error),
    /// A protocol value could not be encoded canonically.
    #[error(transparent)]
    CanonicalJson(#[from] CanonicalJsonError),
    /// Stored JSON no longer matches the versioned evidence model.
    #[error("stored evidence JSON is invalid: {0}")]
    StoredJson(#[from] serde_json::Error),
    /// A content-derived identifier was reused for different bytes.
    #[error("immutable {kind} {id} does not match its stored content")]
    ContentMismatch {
        /// Immutable entity category.
        kind: &'static str,
        /// Reused identifier.
        id: String,
    },
    /// An edge or conflict refers outside its candidate snapshot.
    #[error("candidate snapshot contains an unresolved reference: {detail}")]
    UnresolvedReference {
        /// Relationship validation detail.
        detail: String,
    },
    /// A requested stable slice is larger than its explicit bound.
    #[error("snapshot contains {actual} {kind}, exceeding limit {limit}")]
    SliceLimitExceeded {
        /// Bounded entity category.
        kind: &'static str,
        /// Stored entity count.
        actual: usize,
        /// Requested maximum count.
        limit: usize,
    },
}

/// Metadata fixed when a candidate snapshot begins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateMetadata {
    /// Stable snapshot identifier.
    pub id: String,
    /// Caller-supplied RFC 3339 creation time.
    pub created_at: String,
}

/// Whether one source was fresh when a snapshot was assembled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// The source synchronized successfully for this snapshot.
    Fresh,
    /// The snapshot retained an older valid contribution.
    Stale,
}

impl Freshness {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
        }
    }

    fn from_str(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "fresh" => Ok(Self::Fresh),
            "stale" => Ok(Self::Stale),
            other => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown freshness {other}").into(),
            )),
        }
    }
}

/// Per-source state captured inside a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceState {
    /// Stable source configuration identifier.
    pub source_id: String,
    /// Source system category.
    pub source_kind: SourceKind,
    /// Fresh or stale contribution state.
    pub freshness: Freshness,
    /// Caller-supplied RFC 3339 observation time.
    pub observed_at: String,
    /// Optional resumable synchronization cursor.
    pub cursor: Option<String>,
}

/// Sanitized diagnostic for an input excluded from a candidate snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuarantineDiagnostic {
    /// Repository-relative or source-native locator.
    pub source_locator: String,
    /// Stable diagnostic category.
    pub code: String,
    /// Sanitized reason that excludes source contents and credentials.
    pub message: String,
}

/// Explicit limits for a stable graph read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceLimits {
    /// Maximum evidence records returned.
    pub max_nodes: usize,
    /// Maximum evidence relationships returned.
    pub max_edges: usize,
    /// Maximum explicit conflicts returned.
    pub max_conflicts: usize,
}

/// One transactionally stable evidence graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphSlice {
    /// Published snapshot identifier.
    pub snapshot_id: String,
    /// Canonically ordered evidence records.
    pub nodes: Vec<EvidenceNode>,
    /// Canonically ordered evidence relationships.
    pub edges: Vec<EvidenceEdge>,
    /// Canonically ordered explicit conflicts.
    pub conflicts: Vec<Conflict>,
    /// Canonically ordered source freshness records.
    pub sources: Vec<SourceState>,
}

/// SQLite repository for immutable evidence and atomic snapshot publication.
#[derive(Debug)]
pub struct EvidenceStore {
    connection: Connection,
}

impl EvidenceStore {
    /// Open or create an evidence database and apply known migrations.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` when SQLite cannot open or migrate the database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    /// Create a migrated in-memory evidence database.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` when SQLite cannot initialize the database.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, StoreError> {
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute_batch(INITIAL_MIGRATION)?;
        Ok(Self { connection })
    }

    /// Begin an isolated candidate snapshot using an immediate write transaction.
    ///
    /// Dropping the returned candidate abandons it automatically.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if a write transaction cannot begin or the snapshot
    /// identifier already exists.
    pub fn begin_candidate(
        &mut self,
        metadata: CandidateMetadata,
    ) -> Result<CandidateSnapshot<'_>, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO snapshots (id, created_at, state) VALUES (?1, ?2, 'candidate')",
            params![metadata.id, metadata.created_at],
        )?;
        Ok(CandidateSnapshot {
            transaction,
            snapshot_id: metadata.id,
            created_at: metadata.created_at,
        })
    }

    /// Return the current published snapshot identifier, if one exists.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` when the pointer cannot be read.
    pub fn current_snapshot_id(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT snapshot_id FROM current_snapshot WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Load the complete current graph under one stable read transaction.
    ///
    /// Returns `None` before the first candidate is published.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` for database, decoding, or explicit slice-limit
    /// failures.
    pub fn load_current_slice(
        &mut self,
        limits: SliceLimits,
    ) -> Result<Option<GraphSlice>, StoreError> {
        let transaction = self.connection.transaction()?;
        let snapshot_id: Option<String> = transaction
            .query_row(
                "SELECT snapshot_id FROM current_snapshot WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(snapshot_id) = snapshot_id else {
            transaction.commit()?;
            return Ok(None);
        };

        enforce_limit(
            &transaction,
            "snapshot_records",
            &snapshot_id,
            "nodes",
            limits.max_nodes,
        )?;
        enforce_limit(
            &transaction,
            "snapshot_edges",
            &snapshot_id,
            "edges",
            limits.max_edges,
        )?;
        enforce_limit(
            &transaction,
            "snapshot_conflicts",
            &snapshot_id,
            "conflicts",
            limits.max_conflicts,
        )?;

        let nodes = load_json_rows::<EvidenceNode>(
            &transaction,
            "SELECT records.content_json
             FROM snapshot_records
             JOIN records ON records.id = snapshot_records.record_id
             WHERE snapshot_records.snapshot_id = ?1
             ORDER BY records.id",
            &snapshot_id,
        )?;
        let edges = load_json_rows::<EvidenceEdge>(
            &transaction,
            "SELECT edges.content_json
             FROM snapshot_edges
             JOIN edges ON edges.id = snapshot_edges.edge_id
             WHERE snapshot_edges.snapshot_id = ?1
             ORDER BY edges.id",
            &snapshot_id,
        )?;
        let conflicts = load_json_rows::<Conflict>(
            &transaction,
            "SELECT conflicts.content_json
             FROM snapshot_conflicts
             JOIN conflicts USING (left_node_id, right_node_id, rule_id)
             WHERE snapshot_conflicts.snapshot_id = ?1
             ORDER BY left_node_id, right_node_id, rule_id",
            &snapshot_id,
        )?;
        let sources = load_sources(&transaction, &snapshot_id)?;
        transaction.commit()?;
        Ok(Some(GraphSlice {
            snapshot_id,
            nodes,
            edges,
            conflicts,
            sources,
        }))
    }

    /// Look up the immutable origin for one stored record.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if the database cannot be read or the stored origin
    /// is invalid.
    pub fn record_origin(&self, record_id: &str) -> Result<Option<Origin>, StoreError> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT origin_json FROM records WHERE id = ?1",
                [record_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    /// Load one record only if it belongs to the current published snapshot.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if the current view cannot be read or decoded.
    pub fn current_record(&self, record_id: &str) -> Result<Option<EvidenceNode>, StoreError> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT records.content_json
                 FROM current_snapshot
                 JOIN snapshot_records
                   ON snapshot_records.snapshot_id = current_snapshot.snapshot_id
                 JOIN records ON records.id = snapshot_records.record_id
                 WHERE current_snapshot.singleton = 1 AND records.id = ?1",
                [record_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    /// Inspect source freshness for the current published snapshot.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if the current pointer or source rows cannot be read.
    pub fn current_freshness(&self) -> Result<Vec<SourceState>, StoreError> {
        let Some(snapshot_id) = self.current_snapshot_id()? else {
            return Ok(Vec::new());
        };
        load_sources(&self.connection, &snapshot_id)
    }

    /// Return the last cursor committed with a published snapshot for one source.
    ///
    /// Candidate and abandoned cursors are never visible through this method.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if the cursor table cannot be read.
    pub fn source_cursor(&self, source_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT cursor FROM source_cursors WHERE source_id = ?1",
                [source_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Return sanitized quarantine diagnostics for the current snapshot.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if diagnostic rows cannot be read.
    pub fn current_quarantine(&self) -> Result<Vec<QuarantineDiagnostic>, StoreError> {
        let Some(snapshot_id) = self.current_snapshot_id()? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT source_locator, diagnostic_code, diagnostic_message
             FROM quarantined_inputs
             WHERE snapshot_id = ?1
             ORDER BY id",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok(QuarantineDiagnostic {
                source_locator: row.get(0)?,
                code: row.get(1)?,
                message: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

/// Transactional builder for one candidate evidence snapshot.
#[derive(Debug)]
pub struct CandidateSnapshot<'connection> {
    transaction: Transaction<'connection>,
    snapshot_id: String,
    created_at: String,
}

impl CandidateSnapshot<'_> {
    /// Insert an immutable record idempotently and add it to this snapshot.
    ///
    /// # Errors
    ///
    /// Returns `ContentMismatch` if the identifier already names different
    /// canonical content.
    pub fn insert_record(&self, node: &EvidenceNode) -> Result<(), StoreError> {
        let content = to_canonical_json(node)?;
        let origin = to_canonical_json(&node.origin)?;
        insert_immutable(
            &self.transaction,
            "records",
            &node.id,
            &content,
            Some(&origin),
            &self.created_at,
        )?;
        self.transaction.execute(
            "INSERT OR IGNORE INTO snapshot_records (snapshot_id, record_id) VALUES (?1, ?2)",
            params![self.snapshot_id, node.id],
        )?;
        Ok(())
    }

    /// Insert an immutable edge idempotently and add it to this snapshot.
    ///
    /// Referenced records must already exist in the immutable record table.
    ///
    /// # Errors
    ///
    /// Returns `ContentMismatch` for an ID collision or `Database` for missing
    /// referenced records.
    pub fn insert_edge(&self, edge: &EvidenceEdge) -> Result<(), StoreError> {
        let content = to_canonical_json(edge)?;
        let origin = to_canonical_json(&edge.origin)?;
        let changed = self.transaction.execute(
            "INSERT OR IGNORE INTO edges
             (id, source_id, target_id, content_json, origin_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                edge.id,
                edge.source_id,
                edge.target_id,
                content,
                origin,
                self.created_at
            ],
        )?;
        if changed == 0 {
            ensure_same_content(&self.transaction, "edges", &edge.id, &content)?;
        }
        self.transaction.execute(
            "INSERT OR IGNORE INTO snapshot_edges (snapshot_id, edge_id) VALUES (?1, ?2)",
            params![self.snapshot_id, edge.id],
        )?;
        Ok(())
    }

    /// Insert immutable explicit conflict metadata into this snapshot.
    ///
    /// # Errors
    ///
    /// Returns `ContentMismatch` if an existing conflict key has different
    /// canonical content.
    pub fn insert_conflict(&self, conflict: &Conflict) -> Result<(), StoreError> {
        let content = to_canonical_json(conflict)?;
        let changed = self.transaction.execute(
            "INSERT OR IGNORE INTO conflicts
             (left_node_id, right_node_id, rule_id, content_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                conflict.left_node_id,
                conflict.right_node_id,
                conflict.rule_id,
                content,
                self.created_at
            ],
        )?;
        if changed == 0 {
            let stored: String = self.transaction.query_row(
                "SELECT content_json FROM conflicts
                 WHERE left_node_id = ?1 AND right_node_id = ?2 AND rule_id = ?3",
                params![
                    conflict.left_node_id,
                    conflict.right_node_id,
                    conflict.rule_id
                ],
                |row| row.get(0),
            )?;
            if stored != content {
                return Err(StoreError::ContentMismatch {
                    kind: "conflict",
                    id: format!(
                        "{}:{}:{}",
                        conflict.left_node_id, conflict.right_node_id, conflict.rule_id
                    ),
                });
            }
        }
        self.transaction.execute(
            "INSERT OR IGNORE INTO snapshot_conflicts
             (snapshot_id, left_node_id, right_node_id, rule_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                self.snapshot_id,
                conflict.left_node_id,
                conflict.right_node_id,
                conflict.rule_id
            ],
        )?;
        Ok(())
    }

    /// Record source freshness and an optional cursor for this candidate.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if the source kind cannot be encoded or SQLite
    /// rejects the source state.
    pub fn set_source_state(&self, source: &SourceState) -> Result<(), StoreError> {
        let source_kind = source_kind_name(source.source_kind)?;
        self.transaction.execute(
            "INSERT INTO snapshot_sources
             (snapshot_id, source_id, source_kind, freshness, observed_at, cursor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (snapshot_id, source_id) DO UPDATE SET
                 source_kind = excluded.source_kind,
                 freshness = excluded.freshness,
                 observed_at = excluded.observed_at,
                 cursor = excluded.cursor",
            params![
                self.snapshot_id,
                source.source_id,
                source_kind,
                source.freshness.as_str(),
                source.observed_at,
                source.cursor
            ],
        )?;
        Ok(())
    }

    /// Add a sanitized quarantine diagnostic to this candidate.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if SQLite rejects the diagnostic.
    pub fn quarantine(&self, diagnostic: &QuarantineDiagnostic) -> Result<(), StoreError> {
        self.transaction.execute(
            "INSERT INTO quarantined_inputs
             (snapshot_id, source_locator, diagnostic_code, diagnostic_message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                self.snapshot_id,
                diagnostic.source_locator,
                diagnostic.code,
                diagnostic.message,
                self.created_at
            ],
        )?;
        Ok(())
    }

    /// Atomically publish this complete snapshot and its resumable cursors.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if references are incomplete or any publication
    /// statement fails. The entire candidate remains unpublished on error.
    pub fn publish(self, published_at: &str) -> Result<(), StoreError> {
        validate_candidate(&self.transaction, &self.snapshot_id)?;
        self.transaction.execute(
            "UPDATE snapshots SET state = 'published', published_at = ?2 WHERE id = ?1",
            params![self.snapshot_id, published_at],
        )?;
        self.transaction.execute(
            "INSERT INTO source_cursors (source_id, cursor, snapshot_id, updated_at)
             SELECT source_id, cursor, snapshot_id, ?2
             FROM snapshot_sources
             WHERE snapshot_id = ?1 AND cursor IS NOT NULL
             ON CONFLICT (source_id) DO UPDATE SET
                 cursor = excluded.cursor,
                 snapshot_id = excluded.snapshot_id,
                 updated_at = excluded.updated_at",
            params![self.snapshot_id, published_at],
        )?;
        self.transaction.execute(
            "INSERT INTO current_snapshot (singleton, snapshot_id) VALUES (1, ?1)
             ON CONFLICT (singleton) DO UPDATE SET snapshot_id = excluded.snapshot_id",
            [self.snapshot_id],
        )?;
        self.transaction.commit()?;
        Ok(())
    }

    /// Explicitly abandon this candidate and roll back all of its writes.
    ///
    /// # Errors
    ///
    /// Returns `StoreError` if SQLite cannot roll back the transaction.
    pub fn abandon(self) -> Result<(), StoreError> {
        self.transaction.rollback()?;
        Ok(())
    }
}

fn insert_immutable(
    transaction: &Transaction<'_>,
    table: &'static str,
    id: &str,
    content: &str,
    origin: Option<&str>,
    created_at: &str,
) -> Result<(), StoreError> {
    debug_assert_eq!(table, "records");
    let changed = transaction.execute(
        "INSERT OR IGNORE INTO records (id, content_json, origin_json, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![id, content, origin.unwrap_or("{}"), created_at],
    )?;
    if changed == 0 {
        ensure_same_content(transaction, table, id, content)?;
    }
    Ok(())
}

fn ensure_same_content(
    transaction: &Transaction<'_>,
    table: &'static str,
    id: &str,
    content: &str,
) -> Result<(), StoreError> {
    let query = match table {
        "records" => "SELECT content_json FROM records WHERE id = ?1",
        "edges" => "SELECT content_json FROM edges WHERE id = ?1",
        _ => unreachable!("immutable table is fixed by the repository"),
    };
    let stored: String = transaction.query_row(query, [id], |row| row.get(0))?;
    if stored == content {
        Ok(())
    } else {
        Err(StoreError::ContentMismatch {
            kind: table,
            id: id.to_owned(),
        })
    }
}

fn validate_candidate(transaction: &Transaction<'_>, snapshot_id: &str) -> Result<(), StoreError> {
    let missing_edge: Option<String> = transaction
        .query_row(
            "SELECT edges.id
             FROM snapshot_edges
             JOIN edges ON edges.id = snapshot_edges.edge_id
             WHERE snapshot_edges.snapshot_id = ?1
               AND (
                   NOT EXISTS (
                       SELECT 1 FROM snapshot_records
                       WHERE snapshot_id = ?1 AND record_id = edges.source_id
                   )
                   OR NOT EXISTS (
                       SELECT 1 FROM snapshot_records
                       WHERE snapshot_id = ?1 AND record_id = edges.target_id
                   )
               )
             ORDER BY edges.id
             LIMIT 1",
            [snapshot_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(edge_id) = missing_edge {
        return Err(StoreError::UnresolvedReference {
            detail: format!("edge {edge_id} leaves the snapshot"),
        });
    }

    let missing_conflict: Option<String> = transaction
        .query_row(
            "SELECT rule_id
             FROM snapshot_conflicts
             WHERE snapshot_id = ?1
               AND (
                   NOT EXISTS (
                       SELECT 1 FROM snapshot_records
                       WHERE snapshot_id = ?1
                         AND record_id = snapshot_conflicts.left_node_id
                   )
                   OR NOT EXISTS (
                       SELECT 1 FROM snapshot_records
                       WHERE snapshot_id = ?1
                         AND record_id = snapshot_conflicts.right_node_id
                   )
               )
             ORDER BY rule_id
             LIMIT 1",
            [snapshot_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(rule_id) = missing_conflict {
        return Err(StoreError::UnresolvedReference {
            detail: format!("conflict {rule_id} leaves the snapshot"),
        });
    }
    Ok(())
}

fn enforce_limit(
    transaction: &Transaction<'_>,
    table: &'static str,
    snapshot_id: &str,
    kind: &'static str,
    limit: usize,
) -> Result<(), StoreError> {
    let query = match table {
        "snapshot_records" => "SELECT COUNT(*) FROM snapshot_records WHERE snapshot_id = ?1",
        "snapshot_edges" => "SELECT COUNT(*) FROM snapshot_edges WHERE snapshot_id = ?1",
        "snapshot_conflicts" => "SELECT COUNT(*) FROM snapshot_conflicts WHERE snapshot_id = ?1",
        _ => unreachable!("snapshot membership table is fixed by the repository"),
    };
    let actual: i64 = transaction.query_row(query, [snapshot_id], |row| row.get(0))?;
    let actual = usize::try_from(actual).expect("SQLite count is non-negative");
    if actual > limit {
        Err(StoreError::SliceLimitExceeded {
            kind,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn load_json_rows<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    query: &str,
    snapshot_id: &str,
) -> Result<Vec<T>, StoreError> {
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map([snapshot_id], |row| row.get::<_, String>(0))?;
    rows.map(|json| {
        let json = json?;
        serde_json::from_str(&json).map_err(StoreError::from)
    })
    .collect()
}

fn load_sources(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<SourceState>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT source_id, source_kind, freshness, observed_at, cursor
         FROM snapshot_sources
         WHERE snapshot_id = ?1
         ORDER BY source_id",
    )?;
    let rows = statement.query_map([snapshot_id], |row| {
        let source_kind: String = row.get(1)?;
        Ok(SourceState {
            source_id: row.get(0)?,
            source_kind: parse_source_kind(&source_kind)?,
            freshness: Freshness::from_str(&row.get::<_, String>(2)?)?,
            observed_at: row.get(3)?,
            cursor: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

fn source_kind_name(kind: SourceKind) -> Result<String, StoreError> {
    let json = serde_json::to_value(kind)?;
    Ok(json
        .as_str()
        .expect("source kind always serializes as a string")
        .to_owned())
}

fn parse_source_kind(value: &str) -> Result<SourceKind, rusqlite::Error> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, error.into())
    })
}

#[cfg(test)]
mod tests;
