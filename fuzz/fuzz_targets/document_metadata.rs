#![no_main]

use std::str;

use libfuzzer_sys::fuzz_target;
use rationale_docs::{DocumentIngestor, DocumentInput};

fuzz_target!(|data: &[u8]| {
    let Some((&kind, source)) = data.split_first() else {
        return;
    };
    let Ok(content) = str::from_utf8(source) else {
        return;
    };
    let path = if kind & 1 == 0 {
        "fuzz.md"
    } else {
        "fuzz.rationale.toml"
    };
    let _ = DocumentIngestor::default().ingest([DocumentInput { path, content }]);
});
