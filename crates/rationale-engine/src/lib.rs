//! Offline orchestration across Git, documents, snapshots, and the proof worker.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use rationale_docs::{DocumentIngestor, DocumentInput, IngestDiagnostic, IngestResult};
use rationale_git::{
    CommitEvidence, GitEvidenceError, GitResolver, LocalGitResolver, ReferenceKind, ResolvedTarget,
    TargetSpec, WorkingChange,
};
use rationale_github::{GitHubClient, GitHubError, GitHubRepository, GitHubSyncOutcome, RateLimit};
use rationale_model::{
    Conflict, EdgeKind, EvidenceEdge, EvidenceNode, KernelRequest, KernelResponse, NodeKind,
    Origin, PROTOCOL_VERSION, ProofGoal, RecordStatus, SourceKind,
};
use rationale_protocol::{ProofUnavailable, WorkerConfig, WorkerSupervisor};
use rationale_store::{
    CandidateMetadata, EvidenceStore, Freshness, QuarantineDiagnostic, SliceLimits, SourceState,
    StoreError,
};
use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

mod candidate;

use candidate::{CandidateQuery, rank_candidates};

const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;
const MAX_SCANNED_ENTRIES: usize = 100_000;

/// Failure in local evidence orchestration.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Local Git target or repository resolution failed.
    #[error(transparent)]
    Git(#[from] GitEvidenceError),
    /// GitHub remote identification or client setup failed.
    #[error(transparent)]
    GitHub(#[from] GitHubError),
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
    #[error("no published evidence snapshot; run `rationale sync`")]
    NoSnapshot,
    /// A query is empty or exceeds the supported local contract.
    #[error("invalid query: {detail}")]
    InvalidQuery {
        /// Stable validation detail.
        detail: String,
    },
    /// Local source scanning exceeded an explicit resource bound.
    #[error("local source scan exceeded {limit} entries")]
    ScanLimit {
        /// Configured entry limit.
        limit: usize,
    },
}

/// Result of publishing or reusing one local evidence snapshot.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
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
    /// Source identifiers whose last valid contributions could not be refreshed.
    pub stale_sources: Vec<String>,
    /// GitHub rate metadata from the completed synchronization, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_rate_limit: Option<GitHubRateLimitView>,
}

/// Public, transport-neutral view of GitHub's primary request budget.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct GitHubRateLimitView {
    /// Request ceiling reported by GitHub.
    pub limit: Option<u64>,
    /// Most conservative remaining count observed during the synchronization.
    pub remaining: Option<u64>,
    /// Unix time at which GitHub reports that the budget resets.
    pub reset_at: Option<i64>,
}

impl From<RateLimit> for GitHubRateLimitView {
    fn from(value: RateLimit) -> Self {
        Self {
            limit: value.limit,
            remaining: value.remaining,
            reset_at: value.reset_at,
        }
    }
}

/// Auditable components in candidate scoring version 1.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CandidateScoreComponents {
    /// Whether the complete query exactly matched the record identifier.
    pub exact_identifier: bool,
    /// Number of exact normalized tokens shared with the record, capped at eight.
    pub token_overlap: u16,
    /// Number of exact path segments shared with the record origin, capped at four.
    pub path_segment_overlap: u16,
    /// Bounded age bucket from zero (unknown or old) to four (newest).
    pub source_recency: u16,
}

impl CandidateScoreComponents {
    /// Reconstruct the version 1 score from its public components.
    #[must_use]
    pub fn score_v1(&self) -> u32 {
        u32::from(self.exact_identifier) * 1_000
            + u32::from(self.token_overlap) * 100
            + u32::from(self.path_segment_overlap) * 20
            + u32::from(self.source_recency)
    }
}

/// One ranked suggestion that is never supplied to the proof kernel as a claim.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CandidateView {
    /// Candidate record identifier.
    pub record_id: String,
    /// Version of the deterministic scoring contract.
    pub score_version: u16,
    /// Total score reconstructed from `components`.
    pub score: u32,
    /// Individual bounded score inputs.
    pub components: CandidateScoreComponents,
    /// Fixed-template non-proof reason the record was suggested.
    pub reason: String,
}

/// Source freshness rendered with a proof result.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
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
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
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
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
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
    worker: Arc<tokio::sync::Mutex<Option<WorkerSupervisor>>>,
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
            worker: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    /// Override companion-worker discovery with an explicit executable path.
    #[must_use]
    pub fn with_worker_path(mut self, worker_path: Option<PathBuf>) -> Self {
        self.worker_path = worker_path;
        self.worker = Arc::new(tokio::sync::Mutex::new(None));
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
        let local = self.prepare_local_sync()?;
        let now = jiff::Timestamp::now().to_string();
        let sources = local_sources(&local, &now);
        self.publish_evidence(local.evidence, sources, local.history_complete, None)
    }

    /// Build and atomically publish a combined local and GitHub snapshot.
    ///
    /// GitHub failures retain the last valid forge contribution and mark it
    /// stale. Invalid or missing `origin` configuration remains a caller error.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` for local inputs, repository configuration, client
    /// setup, or snapshot publication failures.
    pub async fn sync(&self) -> Result<SyncReport, EngineError> {
        let client = GitHubClient::from_environment()?;
        self.sync_with_github_client(&client).await
    }

    /// Synchronize with an explicitly configured GitHub client.
    ///
    /// This entry point supports hermetic integration tests without weakening
    /// the default environment-based credential contract.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` for local inputs, repository configuration, or
    /// snapshot publication failures. Remote refresh errors become stale state.
    pub async fn sync_with_github_client(
        &self,
        client: &GitHubClient,
    ) -> Result<SyncReport, EngineError> {
        let resolver = LocalGitResolver::discover(&self.root)?;
        let remote = resolver
            .remote_url("origin")?
            .ok_or_else(|| GitHubError::InvalidRemote {
                detail: "repository has no origin remote".to_owned(),
            })?;
        let repository = GitHubRepository::from_remote(&remote)?;
        let source_id = format!("github:{}", repository.slug());
        let local = self.prepare_local_sync()?;
        let now = jiff::Timestamp::now().to_string();
        let mut sources = local_sources(&local, &now);
        let mut evidence = local.evidence;
        let prior = self.prior_github_contribution(&source_id)?;

        let outcome = client
            .synchronize(
                &repository,
                prior
                    .source
                    .as_ref()
                    .and_then(|source| source.cursor.as_deref()),
            )
            .await;
        let github_rate_limit = match outcome {
            Ok(GitHubSyncOutcome::Updated(update)) => {
                merge_github_evidence(&mut evidence, update.records, update.edges);
                sources.push(SourceState {
                    source_id,
                    source_kind: SourceKind::GitHub,
                    freshness: Freshness::Fresh,
                    observed_at: now,
                    cursor: Some(update.cursor.encode()?),
                });
                Some(update.rate_limit.into())
            }
            Ok(GitHubSyncOutcome::NotModified { cursor, rate_limit }) => {
                merge_github_evidence(&mut evidence, prior.records, prior.edges);
                sources.push(SourceState {
                    source_id,
                    source_kind: SourceKind::GitHub,
                    freshness: Freshness::Fresh,
                    observed_at: now,
                    cursor: Some(cursor.encode()?),
                });
                Some(rate_limit.into())
            }
            Err(_error) => {
                let had_prior = prior.source.is_some();
                merge_github_evidence(&mut evidence, prior.records, prior.edges);
                let prior_source = prior.source;
                sources.push(SourceState {
                    source_id: source_id.clone(),
                    source_kind: SourceKind::GitHub,
                    freshness: Freshness::Stale,
                    observed_at: prior_source
                        .as_ref()
                        .map_or_else(|| now.clone(), |source| source.observed_at.clone()),
                    cursor: prior_source.and_then(|source| source.cursor),
                });
                evidence.diagnostics.push(IngestDiagnostic {
                    source_locator: source_id,
                    code: "github_unavailable".to_owned(),
                    message: if had_prior {
                        "GitHub synchronization failed; retained the last valid contribution"
                            .to_owned()
                    } else {
                        "GitHub synchronization failed; no prior contribution was available"
                            .to_owned()
                    },
                });
                None
            }
        };
        self.publish_evidence(
            evidence,
            sources,
            local.history_complete,
            github_rate_limit.as_ref(),
        )
    }

    fn prepare_local_sync(&self) -> Result<LocalSync, EngineError> {
        let resolver = LocalGitResolver::discover(&self.root)?;
        let history = resolver.commit_history()?;
        let (documents, diagnostics) = collect_documents(&self.root)?;
        let ingested =
            DocumentIngestor::default().ingest(documents.iter().map(|document| DocumentInput {
                path: &document.path,
                content: &document.content,
            }));
        let evidence = prepare_evidence(&history.commits, ingested, diagnostics);
        let head = history
            .commits
            .first()
            .map_or_else(String::new, |commit| commit.id.clone());
        Ok(LocalSync {
            evidence,
            history_complete: history.complete,
            head,
            documents_cursor: local_inputs_id(&history.commits, &documents),
        })
    }

    fn prior_github_contribution(
        &self,
        source_id: &str,
    ) -> Result<PriorGitHubContribution, EngineError> {
        if !self.database_path.is_file() {
            return Ok(PriorGitHubContribution::default());
        }
        let mut store = EvidenceStore::open(&self.database_path)?;
        let Some(slice) = store.load_current_slice(SliceLimits {
            max_nodes: 100_000,
            max_edges: 100_000,
            max_conflicts: 100_000,
        })?
        else {
            return Ok(PriorGitHubContribution::default());
        };
        Ok(PriorGitHubContribution {
            records: slice
                .nodes
                .into_iter()
                .filter(|node| node.origin.source_kind == SourceKind::GitHub)
                .collect(),
            edges: slice
                .edges
                .into_iter()
                .filter(|edge| edge.origin.source_kind == SourceKind::GitHub)
                .collect(),
            source: slice
                .sources
                .into_iter()
                .find(|source| source.source_id == source_id),
        })
    }

    fn publish_evidence(
        &self,
        mut evidence: PreparedEvidence,
        mut sources: Vec<SourceState>,
        history_complete: bool,
        github_rate_limit: Option<&GitHubRateLimitView>,
    ) -> Result<SyncReport, EngineError> {
        evidence.diagnostics.sort();
        evidence.conflicts.sort_by(|left, right| {
            (&left.left_node_id, &left.right_node_id, &left.rule_id).cmp(&(
                &right.left_node_id,
                &right.right_node_id,
                &right.rule_id,
            ))
        });
        sources.sort_by(|left, right| left.source_id.cmp(&right.source_id));
        let snapshot_id = evidence_snapshot_id(&evidence, &sources);
        let stale_sources: Vec<_> = sources
            .iter()
            .filter(|source| source.freshness == Freshness::Stale)
            .map(|source| source.source_id.clone())
            .collect();
        let report = |unchanged| SyncReport {
            snapshot_id: snapshot_id.clone(),
            unchanged,
            records: evidence.records.len(),
            edges: evidence.edges.len(),
            conflicts: evidence.conflicts.len(),
            quarantined: evidence.diagnostics.len(),
            history_complete,
            stale_sources: stale_sources.clone(),
            github_rate_limit: github_rate_limit.cloned(),
        };
        if let Some(parent) = self.database_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut store = EvidenceStore::open(&self.database_path)?;
        if store.current_snapshot_id()?.as_deref() == Some(&snapshot_id) {
            return Ok(report(true));
        }

        let now = jiff::Timestamp::now().to_string();
        let candidate = store.begin_candidate(CandidateMetadata {
            id: snapshot_id.clone(),
            created_at: now.clone(),
        })?;
        for record in evidence.records.values() {
            candidate.insert_record(record)?;
        }
        for edge in evidence.edges.values() {
            candidate.insert_edge(edge)?;
        }
        for conflict in &evidence.conflicts {
            candidate.insert_conflict(conflict)?;
        }
        for source in &sources {
            candidate.set_source_state(source)?;
        }
        for diagnostic in &evidence.diagnostics {
            candidate.quarantine(&QuarantineDiagnostic {
                source_locator: diagnostic.source_locator.clone(),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
            })?;
        }
        candidate.publish(&now)?;
        Ok(report(false))
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
        let candidate_records = slice.nodes.clone();
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
        let supervisor = self.proof_worker().await?;
        let response = supervisor.evaluate(request).await?;
        let candidates =
            candidates_for_response(&response, target, &target_evidence, &candidate_records);
        let freshness = freshness_views(slice.sources);
        Ok(WhyResult {
            target: target.to_owned(),
            response,
            candidates,
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

    /// Rank current candidate records for an explicit user query.
    ///
    /// # Errors
    ///
    /// Returns `EngineError` for an empty query, missing snapshot, or storage
    /// failure.
    pub fn search_candidates(&self, query: &str) -> Result<CandidateSearchResult, EngineError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(EngineError::InvalidQuery {
                detail: "candidate query must not be empty".to_owned(),
            });
        }
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
        let candidates = rank_candidates(
            CandidateQuery {
                exact: query,
                text: query,
                path: None,
            },
            &slice.nodes,
            &BTreeSet::new(),
        );
        Ok(CandidateSearchResult {
            query: query.to_owned(),
            candidates,
            freshness: freshness_views(slice.sources),
        })
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

    async fn proof_worker(&self) -> Result<WorkerSupervisor, EngineError> {
        let mut worker = self.worker.lock().await;
        if let Some(supervisor) = worker.as_ref() {
            return Ok(supervisor.clone());
        }
        let worker_config = match &self.worker_path {
            Some(path) => WorkerConfig::new(path),
            None => WorkerConfig::discover()?,
        };
        let supervisor = WorkerSupervisor::start(worker_config).await?;
        *worker = Some(supervisor.clone());
        Ok(supervisor)
    }
}

/// Structured result of a candidate-only search.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CandidateSearchResult {
    /// Normalized non-empty user query.
    pub query: String,
    /// Deterministically ranked suggestions.
    pub candidates: Vec<CandidateView>,
    /// Source state for the snapshot searched.
    pub freshness: Vec<FreshnessView>,
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

struct LocalSync {
    evidence: PreparedEvidence,
    history_complete: bool,
    head: String,
    documents_cursor: String,
}

#[derive(Default)]
struct PriorGitHubContribution {
    records: Vec<EvidenceNode>,
    edges: Vec<EvidenceEdge>,
    source: Option<SourceState>,
}

fn local_sources(local: &LocalSync, observed_at: &str) -> Vec<SourceState> {
    vec![
        SourceState {
            source_id: "local-git".to_owned(),
            source_kind: SourceKind::Git,
            freshness: if local.history_complete {
                Freshness::Fresh
            } else {
                Freshness::Stale
            },
            observed_at: observed_at.to_owned(),
            cursor: Some(local.head.clone()),
        },
        SourceState {
            source_id: "repository-documents".to_owned(),
            source_kind: SourceKind::Document,
            freshness: Freshness::Fresh,
            observed_at: observed_at.to_owned(),
            cursor: Some(local.documents_cursor.clone()),
        },
    ]
}

fn merge_github_evidence(
    evidence: &mut PreparedEvidence,
    records: Vec<EvidenceNode>,
    edges: Vec<EvidenceEdge>,
) {
    for record in records {
        evidence.records.entry(record.id.clone()).or_insert(record);
    }
    for edge in edges {
        if evidence.records.contains_key(&edge.source_id)
            && evidence.records.contains_key(&edge.target_id)
        {
            evidence.edges.entry(edge.id.clone()).or_insert(edge);
        }
    }
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
            observed_at: jiff::Timestamp::from_second(commit.committed_at)
                .ok()
                .map(|timestamp| timestamp.to_string()),
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

fn local_inputs_id(commits: &[CommitEvidence], documents: &[OwnedDocument]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rationale-local-inputs-v1\0");
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
    format!("inputs:{}", hasher.finalize().to_hex())
}

fn evidence_snapshot_id(evidence: &PreparedEvidence, sources: &[SourceState]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rationale-evidence-snapshot-v2\0");
    for record in evidence.records.values() {
        hash_json(&mut hasher, record);
    }
    for edge in evidence.edges.values() {
        hash_json(&mut hasher, edge);
    }
    for conflict in &evidence.conflicts {
        hash_json(&mut hasher, conflict);
    }
    for diagnostic in &evidence.diagnostics {
        for field in [
            &diagnostic.source_locator,
            &diagnostic.code,
            &diagnostic.message,
        ] {
            hasher.update(field.as_bytes());
            hasher.update(b"\0");
        }
    }
    for source in sources {
        hasher.update(source.source_id.as_bytes());
        hasher.update(b"\0");
        hash_json(&mut hasher, &source.source_kind);
        hasher.update(match source.freshness {
            Freshness::Fresh => b"fresh\0",
            Freshness::Stale => b"stale\0",
        });
        if let Some(cursor) = &source.cursor {
            hasher.update(cursor.as_bytes());
        }
        hasher.update(b"\0");
    }
    format!("snapshot:{}", hasher.finalize().to_hex())
}

fn hash_json(hasher: &mut blake3::Hasher, value: &impl Serialize) {
    let canonical = serde_json::to_vec(value).expect("evidence models must serialize");
    hasher.update(&canonical);
    hasher.update(b"\0");
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

fn freshness_views(sources: Vec<SourceState>) -> Vec<FreshnessView> {
    sources
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
        .collect()
}

fn candidates_for_response(
    response: &KernelResponse,
    target: &str,
    evidence: &ResolvedTarget,
    records: &[EvidenceNode],
) -> Vec<CandidateView> {
    let KernelResponse::Proof { proof, .. } = response else {
        return Vec::new();
    };
    if matches!(
        proof.verdict,
        rationale_model::Verdict::Established | rationale_model::Verdict::Conflicted
    ) {
        return Vec::new();
    }

    let mut excluded: BTreeSet<_> = proof
        .gaps
        .iter()
        .filter_map(|gap| gap.from_node_id.clone())
        .collect();
    let mut text = target.to_owned();
    let path = match evidence {
        ResolvedTarget::Commit { commit, .. } => {
            excluded.insert(commit.id.clone());
            text.push('\n');
            text.push_str(&commit.summary);
            None
        }
        ResolvedTarget::Lines {
            path,
            introduced_by,
            changed_by,
            ..
        } => {
            excluded.extend(introduced_by.iter().map(|item| item.commit_id.clone()));
            for change in changed_by {
                text.push('\n');
                text.push_str(&change.commit.summary);
            }
            Some(path.as_str())
        }
    };
    rank_candidates(
        CandidateQuery {
            exact: target,
            text: &text,
            path,
        },
        records,
        &excluded,
    )
}
