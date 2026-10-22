//! Network protocol, snapshot delta compression, prediction/reconciliation,
//! interpolation and lag compensation. Transport-agnostic.

pub mod browser;
pub mod client;
pub mod conn;
pub mod hitreg;
pub mod interp;
pub mod predict;
pub mod protocol;
pub mod rendezvous;
pub mod snapshot;
pub mod transport;
pub mod wire;

/// Protocol version; bump on any wire-format change.
pub const PROTOCOL_VERSION: u16 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_nonzero() {
        assert_ne!(PROTOCOL_VERSION, 0);
    }
}
