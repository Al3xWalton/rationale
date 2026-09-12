use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::ValueEnum;
use serde::Serialize;

use crate::BenchResult;

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scale {
    Small,
    AvaLike,
    Stress,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct ScaleConfig {
    pub(crate) commits: usize,
    pub(crate) files: usize,
    pub(crate) documents: usize,
}

impl Scale {
    pub(crate) const fn config(self) -> ScaleConfig {
        match self {
            Self::Small => ScaleConfig {
                commits: 25,
                files: 100,
                documents: 10,
            },
            Self::AvaLike => ScaleConfig {
                commits: 400,
                files: 1_000,
                documents: 50,
            },
            Self::Stress => ScaleConfig {
                commits: 1_024,
                files: 5_000,
                documents: 500,
            },
        }
    }
}

pub(crate) struct GeneratedRepository {
    path: PathBuf,
    story_count: usize,
    next_revision: usize,
    target: String,
    keep: bool,
}

impl GeneratedRepository {
    pub(crate) fn create(scale: Scale, keep: bool) -> BenchResult<Self> {
        let config = scale.config();
        let path = env::temp_dir().join(format!(
            "rationale-benchmark-{}-{}",
            std::process::id(),
            scale_name(scale)
        ));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir_all(path.join("src"))?;
        fs::create_dir_all(path.join("governance"))?;

        let story_count = config.documents.saturating_sub(1).max(1);
        write_documents(&path, story_count)?;
        for index in 0..config.files {
            fs::write(source_path(&path, index), "revision 1\n")?;
        }

        let mut repository = Self {
            path,
            story_count,
            next_revision: config.commits + 1,
            target: "src/file_0000.txt:1".to_owned(),
            keep,
        };
        repository.git(&["init", "--quiet"], None)?;
        repository.git(&["config", "user.name", "Rationale Benchmark"], None)?;
        repository.git(
            &["config", "user.email", "benchmark@rationale.invalid"],
            None,
        )?;
        repository.git(&["add", "."], None)?;
        repository.commit(1, None)?;

        for revision in 2..=config.commits {
            let file_index = (revision - 1) % config.files;
            let relative = format!("src/file_{file_index:04}.txt");
            fs::write(
                repository.path.join(&relative),
                format!("revision {revision}\n"),
            )?;
            repository.commit(revision, Some(&relative))?;
            repository.target = format!("{relative}:1");
        }
        Ok(repository)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn head(&self) -> BenchResult<String> {
        self.git(&["rev-parse", "HEAD"], None)
    }

    pub(crate) fn commit_incremental(&mut self) -> BenchResult<()> {
        let revision = self.next_revision;
        let config = self.git(&["ls-files", "src/file_*.txt"], None)?;
        let file_count = config.lines().count();
        let file_index = (revision - 1) % file_count;
        let relative = format!("src/file_{file_index:04}.txt");
        fs::write(self.path.join(&relative), format!("revision {revision}\n"))?;
        self.commit(revision, Some(&relative))?;
        self.target = format!("{relative}:1");
        self.next_revision += 1;
        Ok(())
    }

    fn commit(&self, revision: usize, relative: Option<&str>) -> BenchResult<()> {
        if let Some(relative) = relative {
            self.git(&["add", relative], None)?;
        }
        let story = ((revision - 1) % self.story_count) + 1;
        let message = format!(
            "feat(benchmark): record generated revision {revision}\n\nStory: #{story} | Lineage: {story}.1.0"
        );
        self.git(&["commit", "--quiet", "-m", &message], Some(revision))?;
        Ok(())
    }

    fn git(&self, arguments: &[&str], revision: Option<usize>) -> BenchResult<String> {
        let mut command = Command::new("git");
        command.args(arguments).current_dir(&self.path);
        if let Some(revision) = revision {
            let timestamp = 1_767_225_600_u64 + u64::try_from(revision)?;
            let date = format!("@{timestamp} +0000");
            command.env("GIT_AUTHOR_DATE", &date);
            command.env("GIT_COMMITTER_DATE", &date);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(format!(
                "git {arguments:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

impl Drop for GeneratedRepository {
    fn drop(&mut self) {
        if !self.keep {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }
}

fn source_path(root: &Path, index: usize) -> PathBuf {
    root.join(format!("src/file_{index:04}.txt"))
}

fn write_documents(root: &Path, story_count: usize) -> BenchResult<()> {
    fs::write(
        root.join("governance/ADR-0001.md"),
        "---\nrationale:\n  id: ADR-0001\n  kind: decision\n  status: current\n---\n\n# Generated decision\n",
    )?;
    for story in 1..=story_count {
        fs::write(
            root.join(format!("governance/STORY-{story}.md")),
            format!(
                "---\nrationale:\n  id: STORY-{story}\n  kind: work_item\n  status: accepted\n  documents:\n    - ADR-0001\n---\n\n# Generated story {story}\n"
            ),
        )?;
    }
    Ok(())
}

const fn scale_name(scale: Scale) -> &'static str {
    match scale {
        Scale::Small => "small",
        Scale::AvaLike => "ava-like",
        Scale::Stress => "stress",
    }
}
