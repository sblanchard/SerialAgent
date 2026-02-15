//! Built-in tools for SerialAgent.
//!
//! Implements the exec/process tool pair following OpenClaw semantics:
//! - `exec`: run commands foreground or auto-background after yieldMs
//! - `process`: manage background sessions (list/poll/log/write/kill/clear/remove)

pub mod exec;
pub mod manager;
pub mod process;

pub use manager::ProcessManager;
