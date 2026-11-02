//! Reading tool arguments: string fields, clearable fields and plan statuses.

use serde_json::Value;
use thinkrail_core::{PlanStatus, Result};

use super::{Refusal, refuse};

pub(super) fn string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A string field where `""` clears: absent → untouched, present → set or cleared.
pub(super) fn field(args: &Value, key: &str) -> Option<Option<String>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| (!s.trim().is_empty()).then(|| s.to_owned()))
}

pub(super) fn parse_status(raw: &str) -> Option<PlanStatus> {
    match raw {
        "pending" => Some(PlanStatus::Pending),
        "in_progress" => Some(PlanStatus::InProgress),
        "done" => Some(PlanStatus::Done),
        _ => None,
    }
}

pub(super) fn optional_status(
    args: &Value,
) -> Result<std::result::Result<Option<PlanStatus>, Refusal>> {
    Ok(match args.get("status").and_then(Value::as_str) {
        None => Ok(None),
        Some(raw) => parse_status(raw).map(Some).ok_or_else(|| {
            refuse(format!(
                "Unknown status \"{raw}\"; use pending | in_progress | done."
            ))
        }),
    })
}
