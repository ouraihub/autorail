//! Optional engine telemetry. Counters are reported facts, never inferred from transcript text.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub used_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub percent: Option<f32>,
    pub estimate: bool,
    pub auto_compact: bool,
    /// Inclusive of cache reads/writes; those counters below are subsets, not additions.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// An output subset when the engine reports it separately.
    pub reasoning_tokens: Option<u64>,
    /// The engine's cumulative total, not a second host-side sum.
    pub total_tokens: Option<u64>,
}
