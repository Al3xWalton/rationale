#![no_main]

use libfuzzer_sys::fuzz_target;
use rationale_protocol::decode_frame;

fuzz_target!(|data: &[u8]| {
    let Some((&high, tail)) = data.split_first() else {
        return;
    };
    let Some((&low, frame)) = tail.split_first() else {
        return;
    };
    let max_payload = usize::from(u16::from_be_bytes([high, low]));
    let _ = decode_frame(frame, max_payload);
});
