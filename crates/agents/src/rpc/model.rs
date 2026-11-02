//! Model and thinking-level state of pi-family sessions: `get_state` parsing, switch commands and
//! engine-initiated model events.

use serde_json::{Value, json};
use thinkrail_core::{ModelInfo, Result, ThinkingLevel};

use crate::{
    projection::Projection,
    projection_engine::{LEVELS, level_name},
    session::Live,
};

/// The current model of a pi/OMP `get_state` (or `config_update`) payload, with its thinking level.
pub(crate) fn model(state: &Value) -> Option<ModelInfo> {
    let model = &state["model"];
    let id = model["id"].as_str()?;
    Some(ModelInfo {
        provider: model["provider"].as_str().map(str::to_owned),
        id: id.to_owned(),
        effort: state["thinkingLevel"].as_str().and_then(thinking_level),
    })
}

/// The host level an engine spells `name`; `None` for anything else (OMP `inherit`, DSH's
/// provider-default `""`, unknown future levels).
pub(crate) fn thinking_level(name: &str) -> Option<ThinkingLevel> {
    LEVELS.into_iter().find(|&level| level_name(level) == name)
}

/// The pi-family commands (without `type`/`id`) that switch this session's model: `set_model`,
/// then `set_thinking_level` when a level is given. Nothing is persisted (RPC sends no `persist`).
pub(crate) fn switch_commands(
    provider: &str,
    model_id: &str,
    effort: Option<ThinkingLevel>,
) -> Vec<(&'static str, Value)> {
    let mut commands = vec![(
        "set_model",
        json!({"provider": provider, "modelId": model_id}),
    )];
    if let Some(level) = effort {
        commands.push(("set_thinking_level", json!({"level": level_name(level)})));
    }
    commands
}

/// Switches a pi/OMP session's model through `call`, then re-reads `get_state` so the snapshot
/// shows what the engine actually applied (it clamps the level to what the model supports).
pub(crate) fn switch_model(
    live: &Live,
    call: impl Fn(&str, Value) -> Result<Value>,
    provider: &str,
    model_id: &str,
    effort: Option<ThinkingLevel>,
) -> Result<()> {
    live.ensure_idle()?;
    for (kind, fields) in switch_commands(provider, model_id, effort) {
        call(kind, fields)?;
    }
    let state = call("get_state", json!({}))?;
    live.state.lock().set_model(model(&state));
    Ok(())
}

/// Folds an engine-initiated model change into the snapshot (publishes only on a change):
/// `thinking_level_changed {level | thinkingLevel}` (pi | OMP) and OMP `config_update`.
pub(crate) fn model_event(p: &mut Projection, frame: &Value) {
    if frame["type"] == "config_update" && frame["model"].is_object() {
        p.set_model(model(frame));
        return;
    }
    let level = frame
        .get("thinkingLevel")
        .or_else(|| frame.get("level"))
        .and_then(Value::as_str);
    let Some(level) = level else { return };
    if let Some(current) = p.model() {
        let updated = ModelInfo {
            effort: thinking_level(level),
            ..current.clone()
        };
        p.set_model(Some(updated));
    }
}
