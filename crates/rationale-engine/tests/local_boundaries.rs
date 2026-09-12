//! End-to-end coverage for hostile local document inputs.

use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use rationale_engine::Engine;
use rationale_store::EvidenceStore;

static NEXT_REPOSITORY: AtomicU64 = AtomicU64::new(0);

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let serial = NEXT_REPOSITORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "rationale-engine-boundaries-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary repository should be created");
        let repository = Self { path };
        repository.git(&["init", "--quiet"]);
        repository.git(&["config", "user.name", "Rationale Test"]);
        repository.git(&["config", "user.email", "rationale@example.invalid"]);
        fs::write(repository.path.join("README.md"), "# Fixture\n")
            .expect("fixture should be written");
        repository.git(&["add", "README.md"]);
        repository.git(&["commit", "--quiet", "-m", "chore: initialize fixture"]);
        repository
    }

    fn git(&self, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git {arguments:?} should succeed");
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn invalid_utf8_and_oversized_documents_are_quarantined() {
    let repository = TestRepository::new();
    fs::write(repository.path.join("invalid.md"), [0xff, 0xfe])
        .expect("invalid UTF-8 fixture should be written");
    fs::write(
        repository.path.join("oversized.md"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .expect("oversized fixture should be written");

    let engine = Engine::discover(&repository.path, Some(".rationale/boundaries.db".into()))
        .expect("engine should discover the fixture repository");
    let report = engine
        .sync_local()
        .expect("hostile documents should be quarantined without aborting sync");
    assert_eq!(report.quarantined, 2);

    let store = EvidenceStore::open(engine.database_path())
        .expect("published evidence database should remain readable");
    let diagnostics = store
        .current_quarantine()
        .expect("quarantine diagnostics should be readable");
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        ["invalid_utf8", "file_too_large"]
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains('\u{fffd}'))
    );
}
