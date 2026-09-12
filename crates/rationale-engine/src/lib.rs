//! Offline orchestration across Git, documents, snapshots, and the proof worker.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use rationale_docs::{DocumentIngestor, DocumentInput, IngestDiagnostic, IngestResult};
use rationale_git::{
    CommitEvidence, GitEvidenceError, GitResolver, LocalGitResolver, ReferenceKind, ResolvedTarget,
    TargetSpec, WorkingChange,
};
use rationale_model::{
    Conflict, EdgeKind, EvidenceEdge, EvidenceNode, KernelRequest, KernelResponse, NodeKind,
    Origin, PROTOCOL_VERSION, ProofGoal, RecordStatus, SourceKind,
};
use rationale_protocol::{ProofUnavailable, WorkerConfig, WorkerSupervisor};
use rationale_store::{
    CandidateMetadata, EvidenceStore, Freshness, QuarantineDiagnostic, SliceLimits, SourceState,
    StoreError,
};
use serde::Serialize;
use thiserror::Error;

const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;
const MAX_SCANNED_ENTRIES: usize = 100_000;

/// Failure in local evidence orchestration.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Local Git target or repository resolution failed.
    #[error(transparent)]
    Git(#[from] GitEvidenceError),
    /// SQLite evidence access failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Filesystem discovery or reading failed.
    #[error("local evidence I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// The authoritative proof worker was unavailable.
    #[error(transparent)]
    ProofUnavailable(#[from] ProofUnavailable),
    /// No evidence has been synchronized yet.
    #[error("no published evidence snapshot; run `rationale sync --local`")]
    NoSnapshot,
    /// Local source scanning exceeded an explicit resource bound.
    #[error("local source scan exceeded {limit} entries")]
    ScanLimit {
        /// Configured entry limit.
        limit: usize,
    },
}

/// Result of publishing or reusing one local evidence snapshot.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SyncReport {
    /// Content-derived snapshot identifier.
    pub snapshot_id: String,
    /// True when the current snapshot already represented the same inputs.
    pub unchanged: bool,
    /// Number of normalized records in the snapshot.
    pub records: usize,
    /// Number of explicit relationships in the snapshot.
    pub edges: usize,
    /// Number of explicit conflicts in the snapshot.
    pub conflicts: usize,
    /// Number of quarantined inputs or unresolved local references.
    pub quarantined: usize,
    /// Whether the synchronized Git history was complete.
    pub history_complete: bool,
}

/// Empty in Story 8; reserved for later non-proving ranked evidence.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CandidateView {
    /// Candidate record identifier.
    pub record_id: String,
    /// Auditable non-proof reason the record was suggested.
    pub reason: String,
}

/// Source freshness rendered with a proof result.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FreshnessView {
    /// Stable source identifier.
    pub source_id: String,
    /// Source category.
    pub source_kind: SourceKind,
    /// Fresh or stale state.
    pub freshness: String,
    /// Source observation time.
    pub observed_at: String,
}

/// Complete structured answer for a `why` query.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WhyResult {
    /// Normalized user target.
    pub target: String,
    /// Authoritative OCaml kernel response.
    pub response: KernelResponse,
    /// Non-proving suggestions, empty until deterministic ranking is enabled.
    pub candidates: Vec<CandidateView>,
    /// Source state for the snapshot used by this result.
    pub freshness: Vec<FreshnessView>,
}

impl WhyResult {
    /// Whether at least one source contribution was stale.
    #[must_use]
    pub fn has_stale_source(&self) -> bool {
        self.freshness
            .iter()
            .any(|source| source.freshness == "stale")
    }
}

/// One conservative gap for a changed working-copy path.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ChangedGap {
    /// Repository-relative changed path.
    pub path: String,
    /// Stable non-inference gap category.
    pub code: String,
}

/// Local Rationale engine rooted at one discovered repository.
#[derive(Clone, Debug)]
pub struct Engine {
    root: PathBuf,
    database_path: PathBuf,
    worker_path: Option<PathBuf>,
}

impl Engine {
    /// Discover the repository and select its local evidence database.
    ///
    /// A relative database override is resolved from the repository root.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` when repository discovery fails.
    pub fn discover(
        start: impl AsRef<Path>,
        database_override: Option<PathBuf>,
    ) -> Result<Self, EngineError> {
        let resolver = LocalGitResolver::discover(start)?;
        let root = resolver.repository_root().to_path_buf();
        let database_path = database_override.map_or_else(
            || root.join(".rationale/rationale.db"),
            |path| {
                if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
            },
        );
        Ok(Self {
            root,
            database_path,
            worker_path: None,
        })
    }

    /// Override companion-worker discovery with an explicit executable path.
    #[must_use]
    pub fn with_worker_path(mut self, worker_path: Option<PathBuf>) -> Self {
        self.worker_path = worker_path;
        self
    }

    /// Return the canonical repository root.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.root
    }

    /// Return the selected SQLite database path.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Build and atomically publish a deterministic local-only snapshot.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` for Git, filesystem, normalization, or publication
    /// failures. Malformed individual documents are quarantined in the report.
    pub fn sync_local(&self) -> Result<SyncReport, EngineError> {
        let resolver = LocalGitResolver::discover(&self.root)?;
        let history = resolver.commit_history()?;
        let (documents, diagnostics) = collect_documents(&self.root)?;
        let ingested =
            DocumentIngestor::default().ingest(documents.iter().map(|document| DocumentInput {
                path: &document.path,
                content: &document.content,
            }));
        let PreparedEvidence {
            records,
            edges,
            conflicts,
            mut diagnostics,
        } = prepare_evidence(&history.commits, ingested, diagnostics);

        let snapshot_id = snapshot_id(&history.commits, &documents);
        if let Some(parent) = self.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut store = EvidenceStore::open(&self.database_path)?;
        if store.current_snapshot_id()?.as_deref() == Some(&snapshot_id) {
            return Ok(SyncReport {
                snapshot_id,
                unchanged: true,
                records: records.len(),
                edges: edges.len(),
                conflicts: conflicts.len(),
                quarantined: diagnostics.len(),
                history_complete: history.complete,
            });
        }

        let now = jiff::Timestamp::now().to_string();
        let candidate = store.begin_candidate(CandidateMetadata {
            id: snapshot_id.clone(),
            created_at: now.clone(),
        })?;
        for record in records.values() {
            candidate.insert_record(record)?;
        }
        for edge in edges.values() {
            candidate.insert_edge(edge)?;
        }
        for conflict in &conflicts {
            candidate.insert_conflict(conflict)?;
        }
        let head = history
            .commits
            .first()
            .map_or_else(String::new, |commit| commit.id.clone());
        for source in [
            SourceState {
                source_id: "local-git".to_owned(),
                source_kind: SourceKind::Git,
                freshness: Freshness::Fresh,
                observed_at: now.clone(),
                cursor: Some(head),
            },
            SourceState {
                source_id: "repository-documents".to_owned(),
                source_kind: SourceKind::Document,
                freshness: Freshness::Fresh,
                observed_at: now.clone(),
                cursor: Some(snapshot_id.clone()),
            },
        ] {
            candidate.set_source_state(&source)?;
        }
        diagnostics.sort();
        for diagnostic in &diagnostics {
            candidate.quarantine(&QuarantineDiagnostic {
                source_locator: diagnostic.source_locator.clone(),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
            })?;
        }
        candidate.publish(&now)?;
        Ok(SyncReport {
            snapshot_id,
            unchanged: false,
            records: records.len(),
            edges: edges.len(),
            conflicts: conflicts.len(),
            quarantined: diagnostics.len(),
            history_complete: history.complete,
        })
    }

    /// Resolve a target and evaluate it against the current snapshot.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` for missing snapshots, invalid targets, storage
    /// failures, or proof-worker unavailability.
    pub async fn why(&self, target: &str) -> Result<WhyResult, EngineError> {
        let resolver = LocalGitResolver::discover(&self.root)?;
        let parsed = TargetSpec::from_str(target)?;
        let target_evidence = resolver.resolve(&parsed)?;
        if !self.database_path.is_file() {
            return Err(EngineError::NoSnapshot);
        }
        let mut store = EvidenceStore::open(&self.database_path)?;
        let slice = store
            .load_current_slice(SliceLimits {
                max_nodes: 4_000,
                max_edges: 16_384,
                max_conflicts: 4_096,
            })?
            .ok_or(EngineError::NoSnapshot)?;
        let snapshot_id = slice.snapshot_id.clone();
        let mut nodes: BTreeMap<_, _> = slice
            .nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        let mut edges: BTreeMap<_, _> = slice
            .edges
            .into_iter()
            .map(|edge| (edge.id.clone(), edge))
            .collect();
        let target_id = attach_target(&target_evidence, &mut nodes, &mut edges);
        let request = KernelRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: query_id(&snapshot_id, target),
            snapshot_id,
            goal: ProofGoal {
                target_id,
                anchor_kinds: vec![NodeKind::Decision, NodeKind::WorkItem],
            },
            nodes: nodes.into_values().collect(),
            edges: edges.into_values().collect(),
            conflicts: slice.conflicts,
        };
        let worker_config = match &self.worker_path {
            Some(path) => WorkerConfig::new(path),
            None => WorkerConfig::discover()?,
        };
        let supervisor = WorkerSupervisor::start(worker_config).await?;
        let response = supervisor.evaluate(request).await?;
        let freshness = slice
            .sources
            .into_iter()
            .map(|source| FreshnessView {
                source_id: source.source_id,
                source_kind: source.source_kind,
                freshness: match source.freshness {
                    Freshness::Fresh => "fresh",
                    Freshness::Stale => "stale",
                }
                .to_owned(),
                observed_at: source.observed_at,
            })
            .collect();
        Ok(WhyResult {
            target: target.to_owned(),
            response,
            candidates: Vec::new(),
            freshness,
        })
    }

    /// Load one normalized record from the current snapshot.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` when the evidence database cannot be opened or read.
    pub fn show(&self, record_id: &str) -> Result<Option<EvidenceNode>, EngineError> {
        if !self.database_path.is_file() {
            return Err(EngineError::NoSnapshot);
        }
        let store = EvidenceStore::open(&self.database_path)?;
        Ok(store.current_record(record_id)?)
    }

    /// Report changed paths as explicit working-copy attribution gaps.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` when Git status cannot be read.
    pub fn gaps_changed(&self) -> Result<Vec<ChangedGap>, EngineError> {
        let resolver = LocalGitResolver::discover(&self.root)?;
        Ok(resolver
            .working_changes()?
            .into_iter()
            .map(changed_gap)
            .collect())
    }
}

#[derive(Clone, Debug)]
struct OwnedDocument {
    path: String,
    content: String,
}

struct PreparedEvidence {
    records: BTreeMap<String, EvidenceNode>,
    edges: BTreeMap<String, EvidenceEdge>,
    conflicts: Vec<Conflict>,
    diagnostics: Vec<IngestDiagnostic>,
}

fn prepare_evidence(
    commits: &[CommitEvidence],
    ingested: IngestResult,
    mut diagnostics: Vec<IngestDiagnostic>,
) -> PreparedEvidence {
    diagnostics.extend(ingested.diagnostics);
    let mut records: BTreeMap<_, _> = ingested
        .records
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect();
    for commit in commits {
        records.insert(commit.id.clone(), commit_node(commit));
    }

    let mut proposed_edges = ingested.edges;
    for commit in commits {
        proposed_edges.extend(commit_reference_edges(commit));
    }
    let mut edges = BTreeMap::new();
    for edge in proposed_edges {
        if records.contains_key(&edge.source_id) && records.contains_key(&edge.target_id) {
            edges.insert(edge.id.clone(), edge);
        } else {
            diagnostics.push(IngestDiagnostic {
                source_locator: edge.origin.locator,
                code: "unresolved_reference".to_owned(),
                message: "explicit relationship target is absent from local sources".to_owned(),
            });
        }
    }

    let mut conflicts = Vec::new();
    for conflict in ingested.conflicts {
        if records.contains_key(&conflict.left_node_id)
            && records.contains_key(&conflict.right_node_id)
        {
            conflicts.push(conflict);
        } else {
            diagnostics.push(IngestDiagnostic {
                source_locator: conflict.rule_id,
                code: "unresolved_conflict".to_owned(),
                message: "explicit conflict target is absent from local sources".to_owned(),
            });
        }
    }
    PreparedEvidence {
        records,
        edges,
        conflicts,
        diagnostics,
    }
}

fn collect_documents(
    root: &Path,
) -> Result<(Vec<OwnedDocument>, Vec<IngestDiagnostic>), EngineError> {
    let mut stack = vec![root.to_path_buf()];
    let mut documents = Vec::new();
    let mut diagnostics = Vec::new();
    let mut scanned = 0_usize;
    while let Some(directory) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(directory)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            scanned += 1;
            if scanned > MAX_SCANNED_ENTRIES {
                return Err(EngineError::ScanLimit {
                    limit: MAX_SCANNED_ENTRIES,
                });
            }
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !excluded_directory(&path) {
                    stack.push(path);
                }
                continue;
            }
            let relative = path.strip_prefix(root).expect("walk remains under root");
            let relative = normalized_path(relative)?;
            if !is_document_path(&relative) {
                continue;
            }
            if entry.metadata()?.len() > MAX_DOCUMENT_BYTES {
                diagnostics.push(IngestDiagnostic {
                    source_locator: relative,
                    code: "file_too_large".to_owned(),
                    message: "document exceeds the configured byte limit".to_owned(),
                });
                continue;
            }
            match fs::read_to_string(path) {
                Ok(content) => documents.push(OwnedDocument {
                    path: relative,
                    content,
                }),
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    diagnostics.push(IngestDiagnostic {
                        source_locator: relative,
                        code: "invalid_utf8".to_owned(),
                        message: "document is not valid UTF-8".to_owned(),
                    });
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((documents, diagnostics))
}

fn excluded_directory(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name == ".git" || name == ".rationale" || name == "target" || name == "_build"
    })
}

fn is_document_path(path: &str) -> bool {
    let path = Path::new(path);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() == Some("md") {
        return true;
    }
    matches!(extension.as_deref(), Some("toml" | "yaml" | "yml"))
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.to_ascii_lowercase().ends_with(".rationale"))
}

fn normalized_path(path: &Path) -> Result<String, EngineError> {
    let mut parts = Vec::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_str().ok_or_else(|| GitEvidenceError::InvalidPath {
                detail: "document path is not valid UTF-8".to_owned(),
            })?);
        }
    }
    Ok(parts.join("/"))
}

fn commit_node(commit: &CommitEvidence) -> EvidenceNode {
    EvidenceNode {
        id: commit.id.clone(),
        kind: NodeKind::Change,
        status: RecordStatus::Current,
        subject_id: None,
        outcome_id: None,
        origin: Origin {
            source_kind: SourceKind::Git,
            locator: format!("commit:{}", commit.id),
            revision: Some(commit.id.clone()),
            observed_at: None,
        },
    }
}

fn commit_reference_edges(commit: &CommitEvidence) -> Vec<EvidenceEdge> {
    commit
        .references
        .iter()
        .filter_map(|reference| {
            let kind = match reference.kind {
                ReferenceKind::Issue | ReferenceKind::Story => EdgeKind::ResolvesIssue,
                ReferenceKind::Decision => EdgeKind::Documents,
                ReferenceKind::ForgeUrl => return None,
            };
            let origin = Origin {
                source_kind: SourceKind::Git,
                locator: format!("commit:{}#message", commit.id),
                revision: Some(commit.id.clone()),
                observed_at: None,
            };
            Some(evidence_edge(kind, &commit.id, &reference.value, &origin))
        })
        .collect()
}

fn attach_target(
    target: &ResolvedTarget,
    nodes: &mut BTreeMap<String, EvidenceNode>,
    edges: &mut BTreeMap<String, EvidenceEdge>,
) -> String {
    match target {
        ResolvedTarget::Commit { commit, .. } => {
            nodes
                .entry(commit.id.clone())
                .or_insert_with(|| commit_node(commit));
            attach_commit_references(commit, nodes, edges);
            commit.id.clone()
        }
        ResolvedTarget::Lines {
            path,
            start_line,
            end_line,
            revision,
            introduced_by,
            changed_by,
            ..
        } => {
            for change in changed_by {
                nodes
                    .entry(change.commit.id.clone())
                    .or_insert_with(|| commit_node(&change.commit));
                attach_commit_references(&change.commit, nodes, edges);
            }
            let material = format!("{path}:{start_line}-{end_line}@{revision}");
            let id = format!("code:{}", blake3::hash(material.as_bytes()).to_hex());
            let origin = Origin {
                source_kind: SourceKind::Git,
                locator: format!("{path}:{start_line}-{end_line}"),
                revision: Some(revision.clone()),
                observed_at: None,
            };
            nodes.insert(
                id.clone(),
                EvidenceNode {
                    id: id.clone(),
                    kind: NodeKind::CodeTarget,
                    status: RecordStatus::Current,
                    subject_id: None,
                    outcome_id: None,
                    origin: origin.clone(),
                },
            );
            for attribution in introduced_by {
                nodes
                    .entry(attribution.commit_id.clone())
                    .or_insert_with(|| EvidenceNode {
                        id: attribution.commit_id.clone(),
                        kind: NodeKind::Change,
                        status: RecordStatus::Current,
                        subject_id: None,
                        outcome_id: None,
                        origin: Origin {
                            source_kind: SourceKind::Git,
                            locator: format!("commit:{}", attribution.commit_id),
                            revision: Some(attribution.commit_id.clone()),
                            observed_at: None,
                        },
                    });
                let edge =
                    evidence_edge(EdgeKind::IntroducedBy, &id, &attribution.commit_id, &origin);
                edges.insert(edge.id.clone(), edge);
            }
            id
        }
    }
}

fn attach_commit_references(
    commit: &CommitEvidence,
    nodes: &BTreeMap<String, EvidenceNode>,
    edges: &mut BTreeMap<String, EvidenceEdge>,
) {
    for edge in commit_reference_edges(commit) {
        if nodes.contains_key(&edge.target_id) {
            edges.insert(edge.id.clone(), edge);
        }
    }
}

fn evidence_edge(kind: EdgeKind, source: &str, target: &str, origin: &Origin) -> EvidenceEdge {
    let material = format!("{kind:?}\0{source}\0{target}\0{}", origin.locator);
    EvidenceEdge {
        id: format!("edge:{}", blake3::hash(material.as_bytes()).to_hex()),
        kind,
        source_id: source.to_owned(),
        target_id: target.to_owned(),
        status: RecordStatus::Current,
        origin: origin.clone(),
    }
}

fn snapshot_id(commits: &[CommitEvidence], documents: &[OwnedDocument]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rationale-local-snapshot-v1\0");
    for commit in commits {
        hasher.update(commit.id.as_bytes());
        hasher.update(b"\0");
    }
    for document in documents {
        hasher.update(document.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(document.content.as_bytes());
        hasher.update(b"\0");
    }
    format!("snapshot:{}", hasher.finalize().to_hex())
}

fn query_id(snapshot_id: &str, target: &str) -> String {
    let material = format!("rationale-query-v1\0{snapshot_id}\0{target}");
    format!("query:{}", blake3::hash(material.as_bytes()).to_hex())
}

fn changed_gap(change: WorkingChange) -> ChangedGap {
    let code = match change.kind {
        rationale_git::WorkingChangeKind::Added => "working_copy_unattributed",
        rationale_git::WorkingChangeKind::Modified => "working_copy_modified",
        rationale_git::WorkingChangeKind::Deleted => "working_copy_deleted",
        rationale_git::WorkingChangeKind::Renamed => "working_copy_renamed",
        rationale_git::WorkingChangeKind::Conflicted => "working_copy_conflicted",
    };
    ChangedGap {
        path: change.path,
        code: code.to_owned(),
    }
}
