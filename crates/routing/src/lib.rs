//! Provider routing for engine launches: route resolution and signatures, the loopback gateway
//! and provider probes (agent-settings 04 §4, §5).

mod gateway;
mod probe;
mod router;
mod runtime;
mod signature;
mod url;

pub use gateway::Gateway;
pub use probe::Probe;
pub use router::{GatewayRoutes, Router};
pub use url::preview;

#[cfg(test)]
mod tests;
