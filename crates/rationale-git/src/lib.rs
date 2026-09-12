//! Conservative local Git target and history resolution for Rationale.

use std::path::{Path, PathBuf};

use thiserror::Error;

mod local;
mod references;
mod target;

pub use local::LocalGitResolver;
pub use target::{MAX_TARGET_BYTES, TargetSpec};

/// Failure to resolve a target into explicit Git evidence.
#[derive(Debug, Error)]
pub enum GitEvidenceError {
    /// The user-facing target syntax is invalid.
    #[error("invalid Git target: {detail}")]
    InvalidTarget {
        /// Parse failure detail.
        detail: String,
    },
    /// Repository discovery failed.
    #[error("Git repository discovery failed: {detail}")]
    RepositoryDiscovery {
        /// Discovery failure detail.
        detail: String,
    },
    /// Bare repositories cannot resolve working-copy line targets.
    #[error("bare repositories do not have resolvable working-copy targets")]
    BareRepository,
    /// A path is absolute, traverses upward, enters Git internals, or is not UTF-8.
    #[error("invalid repository-relative path: {detail}")]
    InvalidPath {
        /// Path validation detail.
        detail: String,
    },
    /// A symlinked target leaves the discovered repository root.
    #[error("target path escapes the repository through a symlink: {path}")]
    PathEscape {
        /// Normalized target path.
        path: String,
    },
    /// A target path does not exist at the selected working copy or revision.
    #[error("target path does not exist: {path}")]
    MissingPath {
        /// Normalized target path.
        path: String,
    },
    /// Git cannot resolve the requested revision to a commit.
    #[error("invalid Git revision {revision}: {detail}")]
    InvalidRevision {
        /// Requested revision expression.
        revision: String,
        /// Git resolution detail.
        detail: String,
    },
    /// A line range exceeds the selected file content.
    #[error("line {line} exceeds {line_count} lines in {path}")]
    LineOutOfRange {
        /// Normalized target path.
        path: String,
        /// Requested one-based line.
        line: usize,
        /// Available line count.
        line_count: usize,
    },
    /// A changed or untracked line has no committed attribution.
    #[error("working-copy line {line} in {path} has no committed attribution")]
    WorkingCopyUnattributed {
        /// Normalized target path.
        path: String,
        /// First unattributable one-based line.
        line: usize,
    },
    /// A bounded history walk failed.
    #[error("Git operation failed: {detail}")]
    Git {
        /// Git failure detail.
        detail: String,
    },
}

/// Category of an explicit commit-message reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReferenceKind {
    /// Local or forge issue identifier.
    Issue,
    /// Story identifier.
    Story,
    /// Architecture decision identifier.
    Decision,
    /// Full issue or pull-request URL.
    ForgeUrl,
}

/// One explicit reference copied from commit metadata.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExplicitReference {
    /// Reference category.
    pub kind: ReferenceKind,
    /// Normalized literal identifier or URL.
    pub value: String,
}

/// Stable commit metadata used for evidence records and citations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitEvidence {
    /// Full object identifier.
    pub id: String,
    /// First commit-message line.
    pub summary: String,
    /// Commit time in Unix seconds, copied from Git.
    pub committed_at: i64,
    /// Parent commit identifiers in Git order.
    pub parent_ids: Vec<String>,
    /// Explicit supported references in the commit message.
    pub references: Vec<ExplicitReference>,
}

/// Committed or working-copy state of a resolved line target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetState {
    /// Content was read from a committed tree or a clean working file.
    Committed,
    /// Content was read from a modified index or working file.
    WorkingCopy,
}

/// Blame-backed attribution for one requested line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineAttribution {
    /// One-based line number in the resolved target.
    pub line: usize,
    /// Commit that most recently introduced the blamed line.
    pub commit_id: String,
    /// Rename-aware original repository-relative path reported by Git.
    pub original_path: String,
}

/// One commit that explicitly changed the target path in Git history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathChange {
    /// Commit metadata.
    pub commit: CommitEvidence,
    /// Repository-relative path on the newer side of the change.
    pub path: String,
    /// Older path when Git classified the change as a rename.
    pub renamed_from: Option<String>,
    /// Whether this change deleted the tracked path.
    pub deleted: bool,
}

/// Bounded commit history used during local evidence synchronization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitHistory {
    /// Commits in topological, newest-first order.
    pub commits: Vec<CommitEvidence>,
    /// False when a shallow boundary or configured cap truncated the walk.
    pub complete: bool,
}

/// Category of a changed working-copy path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkingChangeKind {
    /// New index or working-tree path.
    Added,
    /// Modified index or working-tree path.
    Modified,
    /// Deleted index or working-tree path.
    Deleted,
    /// Renamed index or working-tree path.
    Renamed,
    /// Conflicted path.
    Conflicted,
}

/// One normalized changed path in the index or working tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingChange {
    /// Repository-relative UTF-8 path.
    pub path: String,
    /// Coarse change category.
    pub kind: WorkingChangeKind,
}

/// Resolved local Git evidence for a commit or line range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedTarget {
    /// Commit target with copied metadata.
    Commit {
        /// Discovered repository root.
        repository_root: PathBuf,
        /// Resolved commit evidence.
        commit: CommitEvidence,
    },
    /// File line target with blame and change history.
    Lines {
        /// Discovered repository root.
        repository_root: PathBuf,
        /// Normalized repository-relative path.
        path: String,
        /// First requested one-based line.
        start_line: usize,
        /// Last requested one-based line.
        end_line: usize,
        /// Full selected commit identifier.
        revision: String,
        /// Committed or working-copy state.
        state: TargetState,
        /// Per-line blame attributions.
        introduced_by: Vec<LineAttribution>,
        /// Rename-aware commits that changed the path.
        changed_by: Vec<PathChange>,
        /// False when a shallow boundary or configured cap truncated history.
        history_complete: bool,
    },
}

/// Adapter boundary for real or fixture-backed Git target resolution.
pub trait GitResolver {
    /// Return the canonical repository root.
    fn repository_root(&self) -> &Path;

    /// Resolve one parsed target into explicit Git evidence.
    ///
    /// # Errors
    ///
    /// Returns `GitEvidenceError` for invalid, unsafe, missing, or unattributed
    /// targets and for underlying Git failures.
    fn resolve(&self, target: &TargetSpec) -> Result<ResolvedTarget, GitEvidenceError>;
}
