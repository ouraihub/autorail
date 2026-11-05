//! pi-family wire telemetry adapter. Engine totals stay authoritative; cache counts are subsets.

use serde_json::Value;
use thinkrail_core::SessionUsage;

pub(crate) fn parse(state: &Value, stats: &Value) -> SessionUsage {
    // pi reports context in session stats; OMP additionally exposes it in get_state.
    let context = state
        .get("contextUsage")
        .filter(|value| value.is_object())
        .unwrap_or(&stats["contextUsage"]);
    let tokens = &stats["tokens"];
    let cache_read = tokens["cacheRead"].as_u64();
    let cache_write = tokens["cacheWrite"].as_u64();
    let input = tokens["input"].as_u64().and_then(|input| {
        input
            .checked_add(cache_read?)
            .and_then(|input| input.checked_add(cache_write?))
    });
    SessionUsage {
        used_tokens: context["tokens"].as_u64(),
        context_window: context["contextWindow"]
            .as_u64()
            .filter(|window| *window > 0),
        percent: context["percent"]
            .as_f64()
            .filter(|percent| percent.is_finite() && (0.0..=100.0).contains(percent))
            .map(|percent| percent as f32),
        // Both producers use local context estimates/usage anchors, not a new provider bill.
        estimate: context["tokens"].is_number(),
        auto_compact: state["autoCompactionEnabled"] == true,
        input_tokens: input,
        output_tokens: tokens["output"].as_u64(),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        reasoning_tokens: tokens["reasoning"].as_u64(),
        total_tokens: tokens["total"].as_u64(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use serde_json::json;

    #[test]
    fn caches_are_input_subsets_and_missing_counters_remain_unknown() {
        let usage = parse(
            &json!({"autoCompactionEnabled":true}),
            &json!({
                "tokens":{"input":10,"cacheRead":20,"cacheWrite":3,"output":4,"reasoning":2,"total":91},
                "contextUsage":{"tokens":null,"contextWindow":200000,"percent":null}
            }),
        );
        assert_eq!(usage.input_tokens, Some(33));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, Some(2));
        assert_eq!(usage.total_tokens, Some(91));
        assert_eq!(usage.context_window, Some(200000));
        assert_eq!(usage.used_tokens, None);
        assert!(usage.auto_compact);
        let missing = parse(&json!({}), &json!({"tokens":{"input":10}}));
        assert_eq!(missing.input_tokens, None);
        assert_eq!(missing.total_tokens, None);
    }
}
