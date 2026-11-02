//! Model-facing plan text: item and plan lines plus the follow-up nudges appended to results.

use thinkrail_core::{Plan, PlanItem, PlanItemId, PlanStatus};
use thinkrail_plan_review::plan_ops::{self, GroupStatus};

pub(super) fn status_name(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Pending => "pending",
        PlanStatus::InProgress => "in_progress",
        PlanStatus::Done => "done",
    }
}

pub(super) fn format_item(item: &PlanItem) -> String {
    let glyph = match item.status {
        PlanStatus::Pending => "[ ]",
        PlanStatus::InProgress => "[~]",
        PlanStatus::Done => "[x]",
    };
    format!("{glyph} {} — {}", item.title, item.id)
}

pub(super) fn format_plan(plan: &Plan) -> String {
    let mut lines = Vec::new();
    for group in &plan.groups {
        let done = group
            .items
            .iter()
            .filter(|i| i.status == PlanStatus::Done)
            .count();
        let status = match plan_ops::group_status(group) {
            GroupStatus::Pending => "pending",
            GroupStatus::Active => "in_progress",
            GroupStatus::Done => "done",
        };
        lines.push(format!(
            "▸ {} [{status} {done}/{}]",
            group.title,
            group.items.len()
        ));
        lines.extend(group.items.iter().map(|i| format!("  {}", format_item(i))));
    }
    if !plan.loose.is_empty() {
        if !plan.groups.is_empty() {
            lines.push("Loose queue:".into());
        }
        lines.extend(plan.loose.iter().map(format_item));
    }
    lines.join("\n")
}

pub(super) fn consistency_nudge(plan: &Plan) -> Option<String> {
    let mut open = plan_ops::items(plan)
        .filter(|i| i.status != PlanStatus::Done)
        .peekable();
    open.peek()?;
    let mut open = open;
    if open.any(|i| i.status == PlanStatus::InProgress) {
        return None;
    }
    Some("note: nothing is in_progress — flip the step you're working on.".into())
}

pub(super) fn next_open_step<'a>(plan: &'a Plan, id: &PlanItemId) -> Option<&'a PlanItem> {
    let group = plan
        .groups
        .iter()
        .find(|g| g.items.iter().any(|i| &i.id == id))?;
    group.items.iter().find(|i| i.status != PlanStatus::Done)
}

pub(super) fn plan_complete_nudge(plan: &Plan) -> Option<String> {
    let mut items = plan_ops::items(plan).peekable();
    items.peek()?;
    if items.any(|i| i.status != PlanStatus::Done) {
        return None;
    }
    Some(match plan.summary.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => format!("plan complete — a summary from an earlier completion is still here; EXTEND it with todo_plan_summary so it keeps covering EVERYTHING done across the whole plan (carry the earlier points forward, add the new work) — do not rewrite it down to only the latest step. Current summary to build on:\n{existing}"),
        None => "plan complete — write a cumulative overall summary with todo_plan_summary covering EVERYTHING done across all tasks, not just the last step, in PR-body shape (## Summary / ## Changes / ## Testing).".into(),
    })
}

pub(super) fn with_nudges(text: String, nudges: &[Option<String>]) -> String {
    let extra: Vec<&str> = nudges.iter().flatten().map(String::as_str).collect();
    if extra.is_empty() {
        text
    } else {
        format!("{text}\n{}", extra.join("\n"))
    }
}
