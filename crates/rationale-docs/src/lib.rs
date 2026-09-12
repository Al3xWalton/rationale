//! Conservative parsing of explicit rationale documents and verification manifests.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use pulldown_cmark::{Event, Parser, Tag};
use rationale_model::{
    Conflict, EdgeKind, EvidenceEdge, EvidenceNode, NodeKind, Origin, RecordStatus, SourceKind,
};
use serde::Deserialize;

const DEFAULT_MAX_FILE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_DOCUMENTS: usize = 10_000;
type ParseFailure = (&'static str, String);
type ParsedFrontMatter<'a> = (FrontMatterKind, &'a str, &'a str);

/// One repository document supplied to deterministic ingestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentInput<'a> {
    /// Repository-relative source path.
    pub path: &'a str,
    /// UTF-8 source contents.
    pub content: &'a str,
}

/// Configurable literal prefixes used to recognize explicit identifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdPatterns {
    /// Prefixes mapped to work-item records.
    pub work_items: Vec<String>,
    /// Prefixes mapped to decision records.
    pub decisions: Vec<String>,
    /// Prefixes mapped to verification records.
    pub verifications: Vec<String>,
}

impl Default for IdPatterns {
    fn default() -> Self {
        Self {
            work_items: vec!["#".to_owned(), "ISSUE-".to_owned(), "STORY-".to_owned()],
            decisions: vec!["ADR-".to_owned()],
            verifications: vec!["VERIFY-".to_owned()],
        }
    }
}

impl IdPatterns {
    fn kind(&self, id: &str) -> Option<NodeKind> {
        if self
            .decisions
            .iter()
            .any(|prefix| valid_prefixed(id, prefix))
        {
            Some(NodeKind::Decision)
        } else if self
            .verifications
            .iter()
            .any(|prefix| valid_prefixed(id, prefix))
        {
            Some(NodeKind::Verification)
        } else if self
            .work_items
            .iter()
            .any(|prefix| valid_prefixed(id, prefix))
        {
            Some(NodeKind::WorkItem)
        } else {
            None
        }
    }

    fn prefixes(&self) -> Vec<&str> {
        let mut prefixes: Vec<_> = self
            .work_items
            .iter()
            .chain(&self.decisions)
            .chain(&self.verifications)
            .map(String::as_str)
            .collect();
        prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        prefixes.dedup();
        prefixes
    }
}

/// Resource bounds and identifier policy for document ingestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestConfig {
    /// Maximum UTF-8 byte length of one source file.
    pub max_file_bytes: usize,
    /// Maximum number of inputs processed in one batch.
    pub max_documents: usize,
    /// Literal identifier prefixes.
    pub id_patterns: IdPatterns,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_documents: DEFAULT_MAX_DOCUMENTS,
            id_patterns: IdPatterns::default(),
        }
    }
}

/// Sanitized reason one input was excluded from normalized evidence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IngestDiagnostic {
    /// Repository-relative source locator.
    pub source_locator: String,
    /// Stable diagnostic category.
    pub code: String,
    /// Parser or validation reason without source contents.
    pub message: String,
}

/// Deterministic normalized output of one document batch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IngestResult {
    /// Records ordered by stable record identifier.
    pub records: Vec<EvidenceNode>,
    /// Relationships ordered by content-derived edge identifier.
    pub edges: Vec<EvidenceEdge>,
    /// Explicit conflicts in canonical tuple order.
    pub conflicts: Vec<Conflict>,
    /// Quarantine diagnostics in locator and code order.
    pub diagnostics: Vec<IngestDiagnostic>,
}

/// Stateless deterministic document normalizer.
#[derive(Clone, Debug)]
pub struct DocumentIngestor {
    config: IngestConfig,
}

impl DocumentIngestor {
    /// Create an ingestor with explicit bounds and identifier patterns.
    #[must_use]
    pub const fn new(config: IngestConfig) -> Self {
        Self { config }
    }

    /// Parse a batch without aborting valid inputs when another is malformed.
    #[must_use]
    pub fn ingest<'a>(&self, inputs: impl IntoIterator<Item = DocumentInput<'a>>) -> IngestResult {
        let mut inputs: Vec<_> = inputs.into_iter().collect();
        inputs.sort_by(|left, right| left.path.cmp(right.path));
        let mut records = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut conflicts = Vec::new();
        let mut diagnostics = Vec::new();

        for (index, input) in inputs.into_iter().enumerate() {
            if index >= self.config.max_documents {
                diagnostics.push(diagnostic(
                    input.path,
                    "document_limit",
                    "document batch exceeds the configured input limit",
                ));
                continue;
            }
            if input.content.len() > self.config.max_file_bytes {
                diagnostics.push(diagnostic(
                    input.path,
                    "file_too_large",
                    "document exceeds the configured byte limit",
                ));
                continue;
            }
            let path = match normalize_path(input.path) {
                Ok(path) => path,
                Err(message) => {
                    diagnostics.push(diagnostic(input.path, "invalid_path", &message));
                    continue;
                }
            };
            let parsed = if is_verification_manifest(&path) {
                parse_verification(&path, input.content)
            } else if has_extension(&path, "md") {
                parse_markdown(&path, input.content, &self.config.id_patterns)
            } else {
                continue;
            };
            let parsed = match parsed {
                Ok(Some(parsed)) => parsed,
                Ok(None) => continue,
                Err((code, message)) => {
                    diagnostics.push(diagnostic(&path, code, &message));
                    continue;
                }
            };

            if let Some(existing) = records.get(&parsed.node.id) {
                if existing != &parsed.node {
                    diagnostics.push(diagnostic(
                        &path,
                        "duplicate_record",
                        "record identifier is already defined by another document",
                    ));
                    continue;
                }
            } else {
                records.insert(parsed.node.id.clone(), parsed.node);
            }
            for edge in parsed.edges {
                edges.insert(edge.id.clone(), edge);
            }
            conflicts.extend(parsed.conflicts);
        }

        conflicts.sort_by(|left, right| {
            (&left.left_node_id, &left.right_node_id, &left.rule_id).cmp(&(
                &right.left_node_id,
                &right.right_node_id,
                &right.rule_id,
            ))
        });
        conflicts.dedup_by(|left, right| left == right);
        diagnostics.sort();
        IngestResult {
            records: records.into_values().collect(),
            edges: edges.into_values().collect(),
            conflicts,
            diagnostics,
        }
    }
}

impl Default for DocumentIngestor {
    fn default() -> Self {
        Self::new(IngestConfig::default())
    }
}

struct ParsedInput {
    node: EvidenceNode,
    edges: Vec<EvidenceEdge>,
    conflicts: Vec<Conflict>,
}

#[derive(Clone, Copy)]
enum FrontMatterKind {
    Yaml,
    Toml,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum MetadataKind {
    WorkItem,
    Decision,
    Verification,
}

impl From<MetadataKind> for NodeKind {
    fn from(value: MetadataKind) -> Self {
        match value {
            MetadataKind::WorkItem => Self::WorkItem,
            MetadataKind::Decision => Self::Decision,
            MetadataKind::Verification => Self::Verification,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictMetadata {
    with: String,
    rule_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RationaleMetadata {
    id: String,
    #[serde(default)]
    kind: Option<MetadataKind>,
    #[serde(default)]
    subject_id: Option<String>,
    #[serde(default)]
    outcome_id: Option<String>,
    #[serde(default = "default_status")]
    status: String,
    #[serde(default)]
    documents: Vec<String>,
    #[serde(default)]
    supersedes: Vec<String>,
    #[serde(default)]
    verified_by: Vec<String>,
    #[serde(default)]
    conflicts: Vec<ConflictMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationManifest {
    version: u16,
    verification: VerificationMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationMetadata {
    id: String,
    artifact: String,
    targets: Vec<String>,
    #[serde(default = "default_status")]
    status: String,
}

fn default_status() -> String {
    "current".to_owned()
}

fn parse_markdown(
    path: &str,
    source: &str,
    patterns: &IdPatterns,
) -> Result<Option<ParsedInput>, ParseFailure> {
    let Some((kind, metadata, body)) = front_matter(source)? else {
        return Ok(None);
    };
    let Some(metadata) = decode_rationale(kind, metadata)? else {
        return Ok(None);
    };
    normalize_markdown(path, body, patterns, metadata).map(Some)
}

fn decode_rationale(
    kind: FrontMatterKind,
    metadata: &str,
) -> Result<Option<RationaleMetadata>, ParseFailure> {
    let metadata = match kind {
        FrontMatterKind::Yaml => {
            let document: serde_json::Value = serde_saphyr::from_str(metadata)
                .map_err(|_| sanitized("invalid_front_matter", "YAML front matter is invalid"))?;
            let Some(rationale) = document.get("rationale") else {
                return Ok(None);
            };
            serde_json::from_value::<RationaleMetadata>(rationale.clone())
                .map_err(|_| sanitized("invalid_rationale", "rationale metadata is invalid"))?
        }
        FrontMatterKind::Toml => {
            let document: toml::Table = metadata.parse().map_err(|_: toml::de::Error| {
                sanitized("invalid_front_matter", "TOML front matter is invalid")
            })?;
            let Some(rationale) = document.get("rationale") else {
                return Ok(None);
            };
            rationale
                .clone()
                .try_into::<RationaleMetadata>()
                .map_err(|_| sanitized("invalid_rationale", "rationale metadata is invalid"))?
        }
    };
    Ok(Some(metadata))
}

fn normalize_markdown(
    path: &str,
    body: &str,
    patterns: &IdPatterns,
    metadata: RationaleMetadata,
) -> Result<ParsedInput, ParseFailure> {
    validate_id(&metadata.id).map_err(|message| ("invalid_rationale", message))?;
    let node_kind = match metadata.kind {
        Some(kind) => kind.into(),
        None => patterns.kind(&metadata.id).ok_or_else(|| {
            (
                "invalid_rationale",
                "record kind cannot be inferred from its configured identifier prefix".to_owned(),
            )
        })?,
    };
    let status = parse_status(&metadata.status)?;
    let origin = Origin {
        source_kind: SourceKind::Document,
        locator: format!("{path}#rationale"),
        revision: None,
        observed_at: None,
    };
    let node = EvidenceNode {
        id: metadata.id.clone(),
        kind: node_kind,
        status,
        subject_id: metadata.subject_id,
        outcome_id: metadata.outcome_id,
        origin: origin.clone(),
    };

    let mut document_targets: BTreeSet<String> = metadata.documents.into_iter().collect();
    document_targets.extend(markdown_identifiers(body, patterns));
    document_targets.remove(&node.id);
    let mut edges = Vec::new();
    for target in document_targets {
        validate_id(&target).map_err(|message| ("invalid_rationale", message))?;
        edges.push(edge(
            EdgeKind::Documents,
            &node.id,
            &target,
            status,
            &origin,
        ));
    }
    for target in metadata.supersedes {
        validate_id(&target).map_err(|message| ("invalid_rationale", message))?;
        edges.push(edge(
            EdgeKind::Supersedes,
            &node.id,
            &target,
            status,
            &origin,
        ));
    }
    for target in metadata.verified_by {
        validate_id(&target).map_err(|message| ("invalid_rationale", message))?;
        edges.push(edge(
            EdgeKind::VerifiedBy,
            &node.id,
            &target,
            status,
            &origin,
        ));
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);

    let mut conflicts = Vec::new();
    for conflict in metadata.conflicts {
        validate_id(&conflict.with).map_err(|message| ("invalid_rationale", message))?;
        if conflict.rule_id.trim().is_empty() {
            return Err((
                "invalid_rationale",
                "conflict rule_id must not be empty".to_owned(),
            ));
        }
        conflicts.push(Conflict {
            left_node_id: node.id.clone(),
            right_node_id: conflict.with,
            rule_id: conflict.rule_id,
        });
    }
    conflicts.sort_by(|left, right| {
        (&left.left_node_id, &left.right_node_id, &left.rule_id).cmp(&(
            &right.left_node_id,
            &right.right_node_id,
            &right.rule_id,
        ))
    });
    Ok(ParsedInput {
        node,
        edges,
        conflicts,
    })
}

fn parse_verification(path: &str, source: &str) -> Result<Option<ParsedInput>, ParseFailure> {
    let manifest: VerificationManifest = if has_extension(path, "toml") {
        toml::from_str(source).map_err(|_| {
            sanitized(
                "invalid_verification",
                "TOML verification manifest is invalid",
            )
        })?
    } else {
        serde_saphyr::from_str(source).map_err(|_| {
            sanitized(
                "invalid_verification",
                "YAML verification manifest is invalid",
            )
        })?
    };
    if manifest.version != 1 {
        return Err((
            "invalid_verification",
            "verification manifest version must be 1".to_owned(),
        ));
    }
    validate_id(&manifest.verification.id).map_err(|message| ("invalid_verification", message))?;
    if manifest.verification.artifact.trim().is_empty() {
        return Err((
            "invalid_verification",
            "verification artifact must not be empty".to_owned(),
        ));
    }
    if manifest.verification.targets.is_empty() {
        return Err((
            "invalid_verification",
            "verification must name at least one target".to_owned(),
        ));
    }
    let status = parse_status(&manifest.verification.status)
        .map_err(|(_, message)| ("invalid_verification", message))?;
    let origin = Origin {
        source_kind: SourceKind::Verification,
        locator: format!("{path}#verification"),
        revision: None,
        observed_at: None,
    };
    let node = EvidenceNode {
        id: manifest.verification.id.clone(),
        kind: NodeKind::Verification,
        status,
        subject_id: None,
        outcome_id: Some(manifest.verification.artifact),
        origin: origin.clone(),
    };
    let mut edges = Vec::new();
    for target in manifest.verification.targets {
        validate_id(&target).map_err(|message| ("invalid_verification", message))?;
        edges.push(edge(
            EdgeKind::VerifiedBy,
            &target,
            &node.id,
            status,
            &origin,
        ));
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);
    Ok(Some(ParsedInput {
        node,
        edges,
        conflicts: Vec::new(),
    }))
}

fn front_matter(source: &str) -> Result<Option<ParsedFrontMatter<'_>>, ParseFailure> {
    let Some(first_end) = source.find('\n') else {
        return Ok(None);
    };
    let first = source[..first_end].trim_end_matches('\r');
    let kind = match first {
        "---" => FrontMatterKind::Yaml,
        "+++" => FrontMatterKind::Toml,
        _ => return Ok(None),
    };
    let delimiter = first;
    let metadata_start = first_end + 1;
    let mut offset = metadata_start;
    for line in source[metadata_start..].split_inclusive('\n') {
        let line_end = offset + line.len();
        if line.trim_end_matches(['\r', '\n']) == delimiter {
            return Ok(Some((
                kind,
                &source[metadata_start..offset],
                &source[line_end..],
            )));
        }
        offset = line_end;
    }
    Err((
        "invalid_front_matter",
        "front matter has no closing delimiter".to_owned(),
    ))
}

fn markdown_identifiers(source: &str, patterns: &IdPatterns) -> BTreeSet<String> {
    let mut identifiers = BTreeSet::new();
    for event in Parser::new(source) {
        match event {
            Event::Text(text) | Event::Code(text) => {
                identifiers.extend(find_identifiers(&text, patterns));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                identifiers.extend(find_identifiers(&dest_url, patterns));
            }
            _ => {}
        }
    }
    identifiers
}

fn find_identifiers(source: &str, patterns: &IdPatterns) -> Vec<String> {
    let mut identifiers = BTreeSet::new();
    let prefixes = patterns.prefixes();
    for (index, _) in source.char_indices() {
        let rest = &source[index..];
        for prefix in &prefixes {
            if !rest.starts_with(prefix) || !boundary_before(source, index) {
                continue;
            }
            let suffix = &rest[prefix.len()..];
            let suffix_length = suffix
                .char_indices()
                .take_while(|(_, character)| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
                .map(|(offset, character)| offset + character.len_utf8())
                .last()
                .unwrap_or(0);
            if suffix_length == 0 {
                continue;
            }
            let id = &rest[..prefix.len() + suffix_length];
            if valid_prefixed(id, prefix) {
                identifiers.insert(id.to_owned());
            }
        }
    }
    identifiers.into_iter().collect()
}

fn boundary_before(source: &str, index: usize) -> bool {
    index == 0
        || source[..index]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
}

fn valid_prefixed(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
    })
}

fn edge(
    kind: EdgeKind,
    source_id: &str,
    target_id: &str,
    status: RecordStatus,
    origin: &Origin,
) -> EvidenceEdge {
    let material = format!("{kind:?}\0{source_id}\0{target_id}\0{}", origin.locator);
    let hash = blake3::hash(material.as_bytes()).to_hex().to_string();
    EvidenceEdge {
        id: format!("edge:{}", &hash[..24]),
        kind,
        source_id: source_id.to_owned(),
        target_id: target_id.to_owned(),
        status,
        origin: origin.clone(),
    }
}

fn parse_status(status: &str) -> Result<RecordStatus, (&'static str, String)> {
    match status {
        "accepted" | "current" => Ok(RecordStatus::Current),
        "historical" => Ok(RecordStatus::Historical),
        _ => Err((
            "invalid_rationale",
            "status must be accepted, current, or historical".to_owned(),
        )),
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 {
        return Err("identifier must contain between 1 and 128 bytes".to_owned());
    }
    if !id.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '#' | '-' | '_' | '.' | ':')
    }) {
        return Err("identifier contains unsupported characters".to_owned());
    }
    Ok(())
}

fn is_verification_manifest(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".rationale.toml")
        || path.ends_with(".rationale.yaml")
        || path.ends_with(".rationale.yml")
}

fn has_extension(path: &str, expected: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn normalize_path(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) if part != ".git" => {
                let part = part
                    .to_str()
                    .ok_or_else(|| "path is not valid UTF-8".to_owned())?;
                parts.push(part);
            }
            Component::CurDir => {}
            Component::Normal(_) => return Err("path enters Git internals".to_owned()),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path must remain repository-relative".to_owned());
            }
        }
    }
    if parts.is_empty() {
        Err("path is empty after normalization".to_owned())
    } else {
        Ok(parts.join("/"))
    }
}

fn diagnostic(path: &str, code: &str, message: &str) -> IngestDiagnostic {
    IngestDiagnostic {
        source_locator: path.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn sanitized(code: &'static str, message: &str) -> ParseFailure {
    (code, message.to_owned())
}

#[cfg(test)]
mod tests;
