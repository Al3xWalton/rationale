//! Shared domain types for Rationale.
//!
//! The crate begins with the protocol version so the Rust host and OCaml worker
//! can establish an explicit compatibility boundary during bootstrap.

/// First version of the Rust-to-OCaml proof protocol.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::PROTOCOL_VERSION;

    #[test]
    fn protocol_version_starts_at_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
