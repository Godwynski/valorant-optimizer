//! val-opt-core: Core daemon engine, benchmarking pipeline, and optimization drivers.

pub mod benchmarking;
pub mod ipc_server;
pub mod latency;
pub mod network;
pub mod optimizations;
pub mod process;
pub mod safety;
pub mod state;

pub use benchmarking::*;
pub use ipc_server::*;
pub use latency::*;
pub use network::*;
pub use optimizations::*;
pub use process::*;
pub use safety::*;
pub use state::*;
