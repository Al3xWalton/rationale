#![no_main]

use std::{str, str::FromStr};

use libfuzzer_sys::fuzz_target;
use rationale_git::TargetSpec;

fuzz_target!(|data: &[u8]| {
    if let Ok(target) = str::from_utf8(data) {
        let _ = TargetSpec::from_str(target);
    }
});
