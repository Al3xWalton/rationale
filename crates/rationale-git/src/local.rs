use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use git2::{Blame, BlameOptions, Delta, DiffFindOptions, Oid, Repository, Sort, Status};

use crate::{
    CommitEvidence, GitEvidenceError, GitResolver, LineAttribution, PathChange, ResolvedTarget,
    TargetSpec, TargetState, references,
};

const DEFAULT_HISTORY_LIMIT: usize = 1_024;
const MAX_WALKED_COMMITS: usize = 100_000;

/// Real local-repository implementation backed by libgit2.
pub struct LocalGitResolver {
    repository: Repository,
    root: PathBuf,
    history_limit: usize,
}

impl LocalGitResolver {
    /// Discover a repository from a path inside its worktree.
    ///
    /// # Errors
    ///
    /// Returns `GitEvidenceError` when discovery fails, the repository is bare,
    /// or the worktree root cannot be canonicalized.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, GitEvidenceError> {
        let repository =
            Repository::discover(start).map_err(|error| GitEvidenceError::RepositoryDiscovery {
                detail: error.message().to_owned(),
            })?;
        let workdir = repository
            .workdir()
            .ok_or(GitEvidenceError::BareRepository)?;
        let root =
            workdir
                .canonicalize()
                .map_err(|error| GitEvidenceError::RepositoryDiscovery {
                    detail: error.to_string(),
                })?;
        Ok(Self {
            repository,
            root,
            history_limit: DEFAULT_HISTORY_LIMIT,
        })
    }

    /// Set the maximum number of matching path changes returned.
    #[must_use]
    pub fn with_history_limit(mut self, history_limit: usize) -> Self {
        self.history_limit = history_limit.max(1);
        self
    }

    fn resolve_commit(&self, revision: &str) -> Result<git2::Commit<'_>, GitEvidenceError> {
        self.repository
            .revparse_single(revision)
            .and_then(|object| object.peel_to_commit())
            .map_err(|error| GitEvidenceError::InvalidRevision {
                revision: revision.to_owned(),
                detail: error.message().to_owned(),
            })
    }

    fn resolve_lines(
        &self,
        path: &Path,
        start_line: usize,
        end_line: usize,
        revision: Option<&str>,
    ) -> Result<ResolvedTarget, GitEvidenceError> {
        let (relative, normalized) = normalize_path(path)?;
        ensure_within_root(&self.root, &relative, &normalized)?;
        let commit = self.resolve_commit(revision.unwrap_or("HEAD"))?;
        let explicit_revision = revision.is_some();
        let (content, state) = if explicit_revision {
            (
                read_committed_blob(&self.repository, &commit, &relative, &normalized)?,
                TargetState::Committed,
            )
        } else {
            let absolute = self.root.join(&relative);
            if !absolute.is_file() {
                return Err(GitEvidenceError::MissingPath { path: normalized });
            }
            let status = self
                .repository
                .status_file(&relative)
                .map_err(|error| git_error(&error))?;
            if status.intersects(Status::WT_NEW | Status::INDEX_NEW) {
                return Err(GitEvidenceError::WorkingCopyUnattributed {
                    path: normalized,
                    line: start_line,
                });
            }
            let state = if status == Status::CURRENT {
                TargetState::Committed
            } else {
                TargetState::WorkingCopy
            };
            (
                fs::read(absolute).map_err(|error| GitEvidenceError::Git {
                    detail: error.to_string(),
                })?,
                state,
            )
        };
        let line_count = count_lines(&content);
        if end_line > line_count {
            return Err(GitEvidenceError::LineOutOfRange {
                path: normalized,
                line: end_line,
                line_count,
            });
        }

        let mut options = BlameOptions::new();
        options
            .newest_commit(commit.id())
            .min_line(start_line)
            .max_line(end_line)
            .track_copies_same_file(true)
            .track_copies_same_commit_moves(true);
        let committed_blame = self
            .repository
            .blame_file(&relative, Some(&mut options))
            .map_err(|error| git_error(&error))?;
        let adjusted_blame;
        let blame = if state == TargetState::WorkingCopy {
            adjusted_blame = committed_blame
                .blame_buffer(&content)
                .map_err(|error| git_error(&error))?;
            &adjusted_blame
        } else {
            &committed_blame
        };
        let introduced_by = attributions(blame, start_line, end_line, &normalized)?;
        let (changed_by, history_complete) = self.path_history(commit.id(), &relative)?;
        Ok(ResolvedTarget::Lines {
            repository_root: self.root.clone(),
            path: normalized,
            start_line,
            end_line,
            revision: commit.id().to_string(),
            state,
            introduced_by,
            changed_by,
            history_complete,
        })
    }

    fn path_history(
        &self,
        start: Oid,
        initial_path: &Path,
    ) -> Result<(Vec<PathChange>, bool), GitEvidenceError> {
        let mut walk = self
            .repository
            .revwalk()
            .map_err(|error| git_error(&error))?;
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
            .map_err(|error| git_error(&error))?;
        walk.push(start).map_err(|error| git_error(&error))?;
        let mut known_paths = BTreeSet::from([initial_path.to_path_buf()]);
        let mut changes = Vec::new();
        let mut complete = !self.repository.path().join("shallow").is_file()
            && !self.repository.commondir().join("shallow").is_file();

        for (walked, oid) in walk.enumerate() {
            if walked >= MAX_WALKED_COMMITS {
                complete = false;
                break;
            }
            let oid = oid.map_err(|error| git_error(&error))?;
            let commit = self
                .repository
                .find_commit(oid)
                .map_err(|error| git_error(&error))?;
            let tree = commit.tree().map_err(|error| git_error(&error))?;
            let mut matched_path = None;
            let mut renamed_from = None;
            let mut deleted = false;
            let parent_count = commit.parent_count();
            let comparisons = parent_count.max(1);
            for parent_index in 0..comparisons {
                let parent_tree = if parent_count == 0 {
                    None
                } else {
                    Some(
                        commit
                            .parent(parent_index)
                            .and_then(|parent| parent.tree())
                            .map_err(|error| git_error(&error))?,
                    )
                };
                let mut diff = self
                    .repository
                    .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
                    .map_err(|error| git_error(&error))?;
                let mut find = DiffFindOptions::new();
                find.renames(true);
                diff.find_similar(Some(&mut find))
                    .map_err(|error| git_error(&error))?;
                for delta in diff.deltas() {
                    let old_path = delta.old_file().path().map(Path::to_path_buf);
                    let new_path = delta.new_file().path().map(Path::to_path_buf);
                    let relevant = old_path
                        .as_ref()
                        .is_some_and(|path| known_paths.contains(path))
                        || new_path
                            .as_ref()
                            .is_some_and(|path| known_paths.contains(path));
                    if !relevant {
                        continue;
                    }
                    if delta.status() == Delta::Renamed
                        && let Some(old_path) = &old_path
                    {
                        known_paths.insert(old_path.clone());
                        renamed_from = Some(path_string(old_path)?);
                    }
                    deleted |= delta.status() == Delta::Deleted;
                    matched_path = new_path.or(old_path);
                }
            }
            if let Some(path) = matched_path {
                changes.push(PathChange {
                    commit: commit_evidence(&commit),
                    path: path_string(&path)?,
                    renamed_from,
                    deleted,
                });
                if changes.len() >= self.history_limit {
                    complete = false;
                    break;
                }
            }
        }
        Ok((changes, complete))
    }
}

impl GitResolver for LocalGitResolver {
    fn repository_root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, target: &TargetSpec) -> Result<ResolvedTarget, GitEvidenceError> {
        match target {
            TargetSpec::Commit { revision } => {
                let commit = self.resolve_commit(revision)?;
                Ok(ResolvedTarget::Commit {
                    repository_root: self.root.clone(),
                    commit: commit_evidence(&commit),
                })
            }
            TargetSpec::Lines {
                path,
                start_line,
                end_line,
                revision,
            } => self.resolve_lines(path, *start_line, *end_line, revision.as_deref()),
        }
    }
}

fn normalize_path(path: &Path) -> Result<(PathBuf, String), GitEvidenceError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                if component == ".git" {
                    return Err(GitEvidenceError::InvalidPath {
                        detail: "targets inside .git are not evidence files".to_owned(),
                    });
                }
                normalized.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(GitEvidenceError::InvalidPath {
                    detail: "path must remain relative to the repository".to_owned(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(GitEvidenceError::InvalidPath {
            detail: "path is empty after normalization".to_owned(),
        });
    }
    let display = path_string(&normalized)?;
    Ok((normalized, display))
}

fn ensure_within_root(root: &Path, relative: &Path, display: &str) -> Result<(), GitEvidenceError> {
    let mut existing = root.join(relative);
    while !existing.exists() {
        if !existing.pop() || existing == root {
            break;
        }
    }
    let canonical = existing
        .canonicalize()
        .map_err(|error| GitEvidenceError::Git {
            detail: error.to_string(),
        })?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(GitEvidenceError::PathEscape {
            path: display.to_owned(),
        })
    }
}

fn read_committed_blob(
    repository: &Repository,
    commit: &git2::Commit<'_>,
    path: &Path,
    display: &str,
) -> Result<Vec<u8>, GitEvidenceError> {
    let tree = commit.tree().map_err(|error| git_error(&error))?;
    let entry = tree
        .get_path(path)
        .map_err(|_| GitEvidenceError::MissingPath {
            path: display.to_owned(),
        })?;
    let blob = entry
        .to_object(repository)
        .and_then(|object| object.peel_to_blob())
        .map_err(|_| GitEvidenceError::MissingPath {
            path: display.to_owned(),
        })?;
    Ok(blob.content().to_vec())
}

fn count_lines(content: &[u8]) -> usize {
    if content.is_empty() {
        0
    } else {
        content.split_inclusive(|byte| *byte == b'\n').count()
    }
}

fn attributions(
    blame: &Blame<'_>,
    start_line: usize,
    end_line: usize,
    display: &str,
) -> Result<Vec<LineAttribution>, GitEvidenceError> {
    (start_line..=end_line)
        .map(|line| {
            let hunk = blame
                .get_line(line)
                .ok_or_else(|| GitEvidenceError::LineOutOfRange {
                    path: display.to_owned(),
                    line,
                    line_count: end_line.saturating_sub(1),
                })?;
            let commit_id = hunk.final_commit_id();
            if commit_id.is_zero() {
                return Err(GitEvidenceError::WorkingCopyUnattributed {
                    path: display.to_owned(),
                    line,
                });
            }
            Ok(LineAttribution {
                line,
                commit_id: commit_id.to_string(),
                original_path: hunk
                    .path()
                    .map(path_string)
                    .transpose()?
                    .unwrap_or_else(|| display.to_owned()),
            })
        })
        .collect()
}

fn commit_evidence(commit: &git2::Commit<'_>) -> CommitEvidence {
    CommitEvidence {
        id: commit.id().to_string(),
        summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default()).into_owned(),
        committed_at: commit.time().seconds(),
        parent_ids: commit
            .parent_ids()
            .map(|parent| parent.to_string())
            .collect(),
        references: references::extract(&String::from_utf8_lossy(commit.message_bytes())),
    }
}

fn path_string(path: &Path) -> Result<String, GitEvidenceError> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| GitEvidenceError::InvalidPath {
            detail: "path is not valid UTF-8".to_owned(),
        })
}

fn git_error(error: &git2::Error) -> GitEvidenceError {
    GitEvidenceError::Git {
        detail: error.message().to_owned(),
    }
}
