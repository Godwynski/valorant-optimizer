//! Benchmarking, ETW frame ingestion, and statistical evaluation modules.

pub mod ab_runner;
pub mod etw_capture;
pub mod frametimes;
pub mod synthetic;

pub use ab_runner::*;
pub use etw_capture::*;
pub use frametimes::*;
pub use synthetic::*;
