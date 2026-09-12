use std::{path::PathBuf, str::FromStr};

use crate::GitEvidenceError;

/// Maximum UTF-8 byte length accepted by the public target syntax.
pub const MAX_TARGET_BYTES: usize = 4_096;

/// A repository target accepted by local Git resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetSpec {
    /// One Git commit or commit-like revision.
    Commit {
        /// Revision expression resolved by Git.
        revision: String,
    },
    /// One inclusive line range in a repository-relative file.
    Lines {
        /// Unnormalized path supplied by the caller.
        path: PathBuf,
        /// First one-based line.
        start_line: usize,
        /// Last one-based line.
        end_line: usize,
        /// Optional historical revision; absence means the working tree at HEAD.
        revision: Option<String>,
    },
}

impl FromStr for TargetSpec {
    type Err = GitEvidenceError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() > MAX_TARGET_BYTES {
            return Err(GitEvidenceError::InvalidTarget {
                detail: format!("target exceeds {MAX_TARGET_BYTES} bytes"),
            });
        }
        if let Some(revision) = input.strip_prefix("commit:") {
            if revision.is_empty() || revision.chars().any(char::is_whitespace) {
                return Err(GitEvidenceError::InvalidTarget {
                    detail: "commit target requires one non-empty revision".to_owned(),
                });
            }
            return Ok(Self::Commit {
                revision: revision.to_owned(),
            });
        }

        let (path, location) =
            input
                .rsplit_once(':')
                .ok_or_else(|| GitEvidenceError::InvalidTarget {
                    detail: "line target must use path:start[-end][@revision]".to_owned(),
                })?;
        if path.is_empty() {
            return Err(GitEvidenceError::InvalidTarget {
                detail: "line target path is empty".to_owned(),
            });
        }
        let (range, revision) = match location.split_once('@') {
            Some((range, revision)) if !revision.is_empty() => (range, Some(revision.to_owned())),
            Some(_) => {
                return Err(GitEvidenceError::InvalidTarget {
                    detail: "historical line target has an empty revision".to_owned(),
                });
            }
            None => (location, None),
        };
        let (start_line, end_line) = if let Some((start, end)) = range.split_once('-') {
            (parse_line(start)?, parse_line(end)?)
        } else {
            let line = parse_line(range)?;
            (line, line)
        };
        if start_line > end_line {
            return Err(GitEvidenceError::InvalidTarget {
                detail: "line range starts after it ends".to_owned(),
            });
        }
        Ok(Self::Lines {
            path: PathBuf::from(path),
            start_line,
            end_line,
            revision,
        })
    }
}

fn parse_line(value: &str) -> Result<usize, GitEvidenceError> {
    let line = value
        .parse::<usize>()
        .map_err(|_| GitEvidenceError::InvalidTarget {
            detail: format!("invalid one-based line number: {value}"),
        })?;
    if line == 0 {
        Err(GitEvidenceError::InvalidTarget {
            detail: "line numbers are one-based".to_owned(),
        })
    } else {
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, str::FromStr};

    use super::TargetSpec;

    #[test]
    fn parses_lines_ranges_revisions_and_commits() {
        assert_eq!(
            TargetSpec::from_str("src/lib.rs:7").expect("line target should parse"),
            TargetSpec::Lines {
                path: PathBuf::from("src/lib.rs"),
                start_line: 7,
                end_line: 7,
                revision: None,
            }
        );
        assert_eq!(
            TargetSpec::from_str("src/lib.rs:7-9@HEAD~2").expect("historical range should parse"),
            TargetSpec::Lines {
                path: PathBuf::from("src/lib.rs"),
                start_line: 7,
                end_line: 9,
                revision: Some("HEAD~2".to_owned()),
            }
        );
        assert_eq!(
            TargetSpec::from_str("commit:HEAD").expect("commit should parse"),
            TargetSpec::Commit {
                revision: "HEAD".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_ambiguous_or_zero_ranges() {
        for invalid in ["src/lib.rs", "src/lib.rs:0", "src/lib.rs:9-2", "commit:"] {
            assert!(TargetSpec::from_str(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn rejects_oversized_targets_before_parsing_components() {
        let target = "x".repeat(super::MAX_TARGET_BYTES + 1);
        assert!(TargetSpec::from_str(&target).is_err());
    }
}
