//! Network protocol, snapshot delta compression, prediction/reconciliation,
//! interpolation and lag compensation. Transport-agnostic.

/// Protocol version; bump on any wire-format change.
pub const PROTOCOL_VERSION: u16 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_nonzero() {
        assert_ne!(PROTOCOL_VERSION, 0);
    }
}
