use std::{path::Path, process::Command};

use serde::Deserialize;

use crate::BenchResult;

#[derive(Debug, Deserialize)]
pub(crate) struct OcamlResult {
    pub(crate) per_evaluation_ms: f64,
}

pub(crate) fn evaluate(
    executable: &Path,
    fixture: &Path,
    iterations: usize,
) -> BenchResult<OcamlResult> {
    let output = Command::new(executable)
        .arg(fixture)
        .arg(iterations.to_string())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "OCaml benchmark failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}
