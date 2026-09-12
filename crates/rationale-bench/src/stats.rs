use std::{collections::BTreeSet, fs, process::Command, time::Duration};

use serde::Serialize;

use crate::BenchResult;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Distribution {
    pub(crate) samples: usize,
    pub(crate) min_ms: f64,
    pub(crate) median_ms: f64,
    pub(crate) p95_ms: f64,
    pub(crate) max_ms: f64,
}

impl Distribution {
    pub(crate) fn from_durations(durations: &[Duration]) -> Self {
        let mut milliseconds: Vec<_> = durations
            .iter()
            .map(|duration| duration.as_secs_f64() * 1_000.0)
            .collect();
        milliseconds.sort_by(f64::total_cmp);
        Self {
            samples: milliseconds.len(),
            min_ms: rounded(milliseconds[0]),
            median_ms: rounded(percentile(&milliseconds, 1, 2)),
            p95_ms: rounded(percentile(&milliseconds, 95, 100)),
            max_ms: rounded(*milliseconds.last().expect("samples are non-empty")),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct HostMetadata {
    pub(crate) measured_at: String,
    pub(crate) operating_system: String,
    pub(crate) architecture: String,
    pub(crate) cpu: String,
    pub(crate) rustc: String,
    pub(crate) ocaml: String,
    pub(crate) rationale_commit: String,
    pub(crate) rationale_index_tree: String,
    pub(crate) source_state: String,
}

pub(crate) fn host_metadata(root: &std::path::Path) -> HostMetadata {
    HostMetadata {
        measured_at: jiff::Timestamp::now().to_string(),
        operating_system: command_output("uname", &["-srm"]),
        architecture: std::env::consts::ARCH.to_owned(),
        cpu: cpu_name(),
        rustc: command_output("rustc", &["--version"]),
        ocaml: command_output(
            "opam",
            &[
                "exec",
                "--switch=rationale-5.5.1",
                "--",
                "ocamlc",
                "-version",
            ],
        ),
        rationale_commit: command_output_in(root, "git", &["rev-parse", "HEAD"]),
        rationale_index_tree: command_output_in(root, "git", &["write-tree"]),
        source_state: source_state(root),
    }
}

pub(crate) fn resident_tree_kib(root_pid: u32) -> BenchResult<u64> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,rss="])
        .output()?;
    if !output.status.success() {
        return Err("ps could not read resident memory".into());
    }
    let rows: Vec<(u32, u32, u64)> = String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
            ))
        })
        .collect();
    let mut included = BTreeSet::from([root_pid]);
    loop {
        let before = included.len();
        for (pid, parent, _) in &rows {
            if included.contains(parent) {
                included.insert(*pid);
            }
        }
        if included.len() == before {
            break;
        }
    }
    Ok(rows
        .iter()
        .filter(|(pid, _, _)| included.contains(pid))
        .map(|(_, _, rss)| *rss)
        .sum())
}

pub(crate) fn rounded(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn percentile(sorted: &[f64], numerator: usize, denominator: usize) -> f64 {
    let rank = sorted.len().saturating_mul(numerator).div_ceil(denominator);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn cpu_name() -> String {
    let macos = command_output("sysctl", &["-n", "machdep.cpu.brand_string"]);
    if macos != "unavailable" {
        return macos;
    }
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|source| {
            source.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_owned())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn command_output_in(root: &std::path::Path, program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_owned())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn source_state(root: &std::path::Path) -> String {
    let status = command_output_in(
        root,
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    );
    if status == "unavailable" || status.is_empty() {
        "clean".to_owned()
    } else if status
        .lines()
        .all(|line| !line.starts_with("??") && line.as_bytes().get(1) == Some(&b' '))
    {
        "staged_changes".to_owned()
    } else {
        "working_tree_changes".to_owned()
    }
}
