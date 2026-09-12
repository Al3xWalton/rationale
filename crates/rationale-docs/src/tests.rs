use std::fmt::Write as _;

use rationale_model::{EdgeKind, NodeKind, RecordStatus};

use super::{DocumentIngestor, DocumentInput, IngestConfig};

const STORY: &str = r"---
title: Story seven
rationale:
  id: STORY-7
  kind: work_item
  subject_id: document-ingestion
  outcome_id: explicit-rationale-metadata
  status: accepted
  documents:
    - ADR-0001
  verified_by:
    - VERIFY-7
  conflicts:
    - with: STORY-OLD
      rule_id: conflict.explicit
---

# Story 7

This work follows [ADR-0001](../decisions/ADR-0001.md) and supersedes no prose.
See ISSUE-42 for the tracked customer requirement.
";

const DECISION: &str = r#"+++
title = "Language boundary"

[rationale]
id = "ADR-0001"
kind = "decision"
status = "current"
subject_id = "runtime-language-boundary"
outcome_id = "rust-host-ocaml-worker-v1"
supersedes = ["ADR-0000"]
+++

# Decision
"#;

const VERIFICATION: &str = r#"version = 1

[verification]
id = "VERIFY-7"
artifact = "reports/story-7.json"
targets = ["ADR-0001"]
status = "current"
"#;

#[test]
fn ingests_namespaced_markdown_and_explicit_relationships() {
    let result = DocumentIngestor::default().ingest([
        DocumentInput {
            path: "governance/STORY-7.md",
            content: STORY,
        },
        DocumentInput {
            path: "docs/ADR-0001.md",
            content: DECISION,
        },
        DocumentInput {
            path: "verification/story-7.rationale.toml",
            content: VERIFICATION,
        },
    ]);
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.records.len(), 3);
    assert!(
        result
            .records
            .iter()
            .any(|record| record.id == "STORY-7" && record.kind == NodeKind::WorkItem)
    );
    assert!(
        result
            .records
            .iter()
            .any(|record| record.id == "ADR-0001" && record.kind == NodeKind::Decision)
    );
    assert!(result.records.iter().any(|record| {
        record.id == "VERIFY-7"
            && record.kind == NodeKind::Verification
            && record.outcome_id.as_deref() == Some("reports/story-7.json")
    }));
    assert!(result.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Documents
            && edge.source_id == "STORY-7"
            && edge.target_id == "ADR-0001"
    }));
    assert!(result.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Documents
            && edge.source_id == "STORY-7"
            && edge.target_id == "ISSUE-42"
    }));
    assert!(result.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Supersedes
            && edge.source_id == "ADR-0001"
            && edge.target_id == "ADR-0000"
    }));
    assert!(result.edges.iter().any(|edge| {
        edge.kind == EdgeKind::VerifiedBy
            && edge.source_id == "ADR-0001"
            && edge.target_id == "VERIFY-7"
    }));
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].left_node_id, "STORY-7");
    assert_eq!(result.conflicts[0].right_node_id, "STORY-OLD");
}

#[test]
fn historical_status_is_preserved() {
    let source = STORY.replace("status: accepted", "status: historical");
    let result = DocumentIngestor::default().ingest([DocumentInput {
        path: "story.md",
        content: &source,
    }]);
    assert_eq!(result.records[0].status, RecordStatus::Historical);
    assert!(
        result
            .edges
            .iter()
            .all(|edge| edge.status == RecordStatus::Historical)
    );
}

#[test]
fn malformed_and_oversized_inputs_are_quarantined_without_aborting_batch() {
    let config = IngestConfig {
        max_file_bytes: STORY.len() + 1,
        ..IngestConfig::default()
    };
    let oversized = "x".repeat(STORY.len() + 2);
    let result = DocumentIngestor::new(config).ingest([
        DocumentInput {
            path: "valid.md",
            content: STORY,
        },
        DocumentInput {
            path: "broken.md",
            content: "---\nrationale: [\n---\n",
        },
        DocumentInput {
            path: "large.md",
            content: &oversized,
        },
    ]);
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.diagnostics.len(), 2);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_front_matter")
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "file_too_large")
    );
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("rationale: ["))
    );
}

#[test]
fn deeply_nested_structured_metadata_is_rejected_before_decoding() {
    let mut yaml_nesting = String::new();
    for depth in 1..=33 {
        writeln!(yaml_nesting, "{}level_{depth}:", "  ".repeat(depth))
            .expect("writing to a string should succeed");
    }
    let yaml = format!(
        "---\nrationale:\n{yaml_nesting}{}value: true\n---\n",
        "  ".repeat(34)
    );
    let toml = format!(
        "version = 1\n[verification]\nid = 'VERIFY-1'\nartifact = 'report'\ntargets = {}'ADR-1'{}\n",
        "[".repeat(33),
        "]".repeat(33)
    );
    let result = DocumentIngestor::default().ingest([
        DocumentInput {
            path: "deep.md",
            content: &yaml,
        },
        DocumentInput {
            path: "deep.rationale.toml",
            content: &toml,
        },
    ]);
    assert!(result.records.is_empty());
    assert_eq!(result.diagnostics.len(), 2);
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "structured_nesting")
    );
}

#[test]
fn verification_requires_artifact_and_target() {
    for invalid in [
        "version = 1\n[verification]\nid = 'VERIFY-1'\nartifact = ''\ntargets = ['ADR-1']\n",
        "version = 1\n[verification]\nid = 'VERIFY-1'\nartifact = 'report.json'\ntargets = []\n",
    ] {
        let result = DocumentIngestor::default().ingest([DocumentInput {
            path: "invalid.rationale.toml",
            content: invalid,
        }]);
        assert!(result.records.is_empty());
        assert_eq!(result.diagnostics[0].code, "invalid_verification");
    }
}

#[test]
fn nearby_test_file_never_creates_verification() {
    let result = DocumentIngestor::default().ingest([
        DocumentInput {
            path: "docs/ADR-0001.md",
            content: DECISION,
        },
        DocumentInput {
            path: "tests/adr_0001_test.md",
            content: "# ADR-0001 tests\nAll tests pass.\n",
        },
    ]);
    assert!(
        result
            .edges
            .iter()
            .all(|edge| edge.kind != EdgeKind::VerifiedBy)
    );
}

#[test]
fn input_order_does_not_change_output() {
    let left = DocumentInput {
        path: "story.md",
        content: STORY,
    };
    let right = DocumentInput {
        path: "decision.md",
        content: DECISION,
    };
    let forward = DocumentIngestor::default().ingest([left, right]);
    let reverse = DocumentIngestor::default().ingest([right, left]);
    assert_eq!(forward, reverse);
}

#[test]
fn non_namespaced_front_matter_is_ignored() {
    let result = DocumentIngestor::default().ingest([DocumentInput {
        path: "README.md",
        content: "---\ntitle: README\nstatus: current\n---\nADR-0001\n",
    }]);
    assert_eq!(result, super::IngestResult::default());
}
