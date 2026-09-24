//! State Snapshot & Disaster Recovery Subsystem.
//!
//! Provides atomic, SHA-256 integrity-verified snapshot persistence
//! and automated Windows boot crash recovery.

pub mod crash_recovery;
pub mod snapshot_engine;

pub use crash_recovery::*;
pub use snapshot_engine::*;
