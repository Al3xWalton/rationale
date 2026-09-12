//! Integration tests against generated local Git repositories.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use rationale_git::{
    GitEvidenceError, GitResolver, LocalGitResolver, ReferenceKind, ResolvedTarget, TargetSpec,
    TargetState,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn init(name: &str) -> Self {
        let repository = Self::empty(name);
        repository.git(["init", "--initial-branch=main"]);
        repository.git(["config", "user.name", "Rationale Test"]);
        repository.git(["config", "user.email", "rationale@example.invalid"]);
        repository
    }

    fn empty(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "rationale-git-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary repository directory should be unique");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent should be created");
        }
        fs::write(path, content).expect("fixture file should be written");
    }

    fn git<const N: usize>(&self, arguments: [&str; N]) -> String {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("git should be installed");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output should be UTF-8")
            .trim()
            .to_owned()
    }

    fn commit(&self, message: &str) -> String {
        self.git(["add", "--all"]);
        self.git(["commit", "--message", message]);
        self.git(["rev-parse", "HEAD"])
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn resolve(repository: &TestRepository, target: &str) -> Result<ResolvedTarget, GitEvidenceError> {
    let resolver = LocalGitResolver::discover(repository.path())?;
    resolver.resolve(&TargetSpec::from_str(target)?)
}

#[test]
fn blame_and_history_cover_initial_and_later_changes() {
    let repository = TestRepository::init("history");
    repository.write("src/app.txt", "alpha\nbeta\ngamma\n");
    let initial = repository.commit("feat(app): add fixture\n\nStory: #1");
    repository.write("src/app.txt", "alpha\nbeta changed\ngamma\n");
    let changed = repository.commit("fix(app): revise beta\n\nDecision: ADR-0007");

    let nested = repository.path().join("src");
    let resolver = LocalGitResolver::discover(&nested).expect("nested discovery should work");
    assert_eq!(
        resolver.repository_root(),
        repository
            .path()
            .canonicalize()
            .expect("root should canonicalize")
    );
    let result = resolver
        .resolve(&TargetSpec::from_str("src/app.txt:2").expect("target should parse"))
        .expect("changed line should resolve");
    let ResolvedTarget::Lines {
        introduced_by,
        changed_by,
        state,
        history_complete,
        ..
    } = result
    else {
        panic!("expected line evidence");
    };
    assert_eq!(state, TargetState::Committed);
    assert!(history_complete);
    assert_eq!(introduced_by[0].commit_id, changed);
    assert!(changed_by.iter().any(|change| change.commit.id == changed));
    assert!(changed_by.iter().any(|change| change.commit.id == initial));
    assert!(changed_by.iter().any(|change| {
        change.commit.references.iter().any(|reference| {
            reference.kind == ReferenceKind::Decision && reference.value == "ADR-0007"
        })
    }));

    let first_line = resolver
        .resolve(&TargetSpec::from_str("src/app.txt:1").expect("target should parse"))
        .expect("unchanged line should resolve");
    let ResolvedTarget::Lines { introduced_by, .. } = first_line else {
        panic!("expected line evidence");
    };
    assert_eq!(introduced_by[0].commit_id, initial);
}

#[test]
fn reads_configured_remote_without_network_access() {
    let repository = TestRepository::init("remote");
    repository.git(["remote", "add", "origin", "git@github.com:acme/widget.git"]);
    let resolver = LocalGitResolver::discover(repository.path()).expect("resolver should open");
    assert_eq!(
        resolver
            .remote_url("origin")
            .expect("remote configuration should be readable")
            .as_deref(),
        Some("git@github.com:acme/widget.git")
    );
    assert_eq!(
        resolver
            .remote_url("missing")
            .expect("missing remote should be explicit"),
        None
    );
}

#[test]
fn working_copy_rejects_new_lines_but_attributes_unchanged_lines() {
    let repository = TestRepository::init("working-copy");
    repository.write("app.txt", "stable\noriginal\n");
    repository.commit("feat(app): add source");
    repository.write("app.txt", "stable\nworking copy\n");

    let error = resolve(&repository, "app.txt:2")
        .expect_err("changed working line should have no committed attribution");
    assert!(matches!(
        error,
        GitEvidenceError::WorkingCopyUnattributed { line: 2, .. }
    ));
    let result = resolve(&repository, "app.txt:1").expect("unchanged line should resolve");
    assert!(matches!(
        result,
        ResolvedTarget::Lines {
            state: TargetState::WorkingCopy,
            ..
        }
    ));

    repository.write("untracked.txt", "new\n");
    let error = resolve(&repository, "untracked.txt:1")
        .expect_err("untracked line should have no attribution");
    assert!(matches!(
        error,
        GitEvidenceError::WorkingCopyUnattributed { line: 1, .. }
    ));
}

#[test]
fn rename_and_deleted_file_remain_resolvable_at_history() {
    let repository = TestRepository::init("rename");
    repository.write("old.txt", "rationale\n");
    repository.commit("feat(file): add old path");
    repository.git(["mv", "old.txt", "new.txt"]);
    let renamed_at = repository.commit("refactor(file): rename source");

    let result = resolve(&repository, "new.txt:1").expect("renamed path should resolve");
    let ResolvedTarget::Lines {
        introduced_by,
        changed_by,
        ..
    } = result
    else {
        panic!("expected line evidence");
    };
    assert_eq!(introduced_by[0].original_path, "old.txt");
    assert!(changed_by.iter().any(|change| {
        change.commit.id == renamed_at && change.renamed_from.as_deref() == Some("old.txt")
    }));

    repository.git(["rm", "new.txt"]);
    repository.commit("refactor(file): remove renamed source");
    assert!(matches!(
        resolve(&repository, "new.txt:1"),
        Err(GitEvidenceError::MissingPath { .. })
    ));
    let historical = format!("new.txt:1@{renamed_at}");
    assert!(matches!(
        resolve(&repository, &historical).expect("historical path should resolve"),
        ResolvedTarget::Lines {
            state: TargetState::Committed,
            ..
        }
    ));
}

#[test]
fn merge_commit_and_explicit_commit_target_are_preserved() {
    let repository = TestRepository::init("merge");
    repository.write("app.txt", "base\n");
    repository.commit("feat(app): add base");
    repository.git(["checkout", "-b", "feature"]);
    repository.write("app.txt", "feature\n");
    repository.commit("feat(app): add feature");
    repository.git(["checkout", "main"]);
    repository.write("other.txt", "main\n");
    repository.commit("feat(other): diverge main");
    repository.git([
        "merge",
        "--no-ff",
        "feature",
        "--message",
        "merge feature #9",
    ]);
    let merge = repository.git(["rev-parse", "HEAD"]);

    let result = resolve(&repository, "commit:HEAD").expect("commit target should resolve");
    let ResolvedTarget::Commit { commit, .. } = result else {
        panic!("expected commit evidence");
    };
    assert_eq!(commit.id, merge);
    assert_eq!(commit.parent_ids.len(), 2);
    assert!(
        commit
            .references
            .iter()
            .any(|reference| reference.value == "#9")
    );

    let lines = resolve(&repository, "app.txt:1").expect("merged line should resolve");
    let ResolvedTarget::Lines { changed_by, .. } = lines else {
        panic!("expected line evidence");
    };
    assert!(changed_by.iter().any(|change| change.commit.id == merge));
}

#[cfg(unix)]
#[test]
fn paths_outside_repository_and_symlink_escapes_fail_closed() {
    use std::os::unix::fs::symlink;

    let repository = TestRepository::init("escape");
    repository.write("inside.txt", "safe\n");
    repository.commit("feat(file): add safe path");
    symlink("/etc/hosts", repository.path().join("escape.txt"))
        .expect("escape symlink should be created");

    assert!(matches!(
        resolve(&repository, "escape.txt:1"),
        Err(GitEvidenceError::PathEscape { .. })
    ));
    assert!(matches!(
        resolve(&repository, "../outside.txt:1"),
        Err(GitEvidenceError::InvalidPath { .. })
    ));
    let absolute = format!("{}:1", repository.path().join("inside.txt").display());
    assert!(matches!(
        resolve(&repository, &absolute),
        Err(GitEvidenceError::InvalidPath { .. })
    ));
}

#[test]
fn invalid_revision_and_line_range_fail_closed() {
    let repository = TestRepository::init("invalid");
    repository.write("app.txt", "one\n");
    repository.commit("feat(app): add source");
    assert!(matches!(
        resolve(&repository, "app.txt:1@does-not-exist"),
        Err(GitEvidenceError::InvalidRevision { .. })
    ));
    assert!(matches!(
        resolve(&repository, "app.txt:2"),
        Err(GitEvidenceError::LineOutOfRange { .. })
    ));
}

#[test]
fn shallow_repository_marks_history_incomplete() {
    let origin = TestRepository::init("shallow-origin");
    origin.write("app.txt", "one\n");
    origin.commit("feat(app): first");
    origin.write("app.txt", "two\n");
    origin.commit("feat(app): second");

    let clone = TestRepository::empty("shallow-clone");
    let source = format!("file://{}", origin.path().display());
    let output = Command::new("git")
        .args(["clone", "--depth", "1", &source])
        .arg(clone.path())
        .output()
        .expect("git should be installed");
    assert!(
        output.status.success(),
        "shallow clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result = resolve(&clone, "app.txt:1").expect("shallow target should resolve");
    assert!(matches!(
        result,
        ResolvedTarget::Lines {
            history_complete: false,
            ..
        }
    ));
}
