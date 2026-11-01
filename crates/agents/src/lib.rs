//! pi / OMP / DSH official process protocols: one `AgentRuntime` per backend.
//!
//! `process`, `framing`, `projection` and `session` are shared by all three; `rpc` is shared by the
//! pi-family backends (pi, OMP); `projection_engine` renders `EngineRouting` into each engine's
//! files and parameters, and `omp_mcp` merges OMP's `mcp.json`. Backend modules never call each
//! other.

mod dsh;
mod framing;
mod host;
mod install;
mod omp;
mod omp_mcp;
mod pi;
mod process;
mod projection;
mod projection_engine;
mod rpc;
mod session;
mod workflow;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod smoke;

pub use dsh::DshRuntime;
pub use install::ManagedEngines;
pub use omp::OmpRuntime;
pub use pi::PiRuntime;
pub use workflow::skill_command;
