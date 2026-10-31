//! pi / OMP / DSH official process protocols: one `AgentRuntime` per backend.
//!
//! `process`, `framing`, `projection` and `session` are shared by all three; `rpc` is shared by the
//! pi-family backends (pi, OMP). Backend modules never call each other.

mod dsh;
mod framing;
mod host;
mod omp;
mod pi;
mod process;
mod projection;
mod rpc;
mod session;
#[cfg(test)]
mod smoke;

pub use dsh::DshRuntime;
pub use omp::OmpRuntime;
pub use pi::PiRuntime;
