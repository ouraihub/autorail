//! The `todo_*` plan tools: listing, adding, updating, removing, promoting, writing and summarising.

use serde_json::Value;
use thinkrail_core::{PlanItemId, PlanScope, PlanStatus, Result};
use thinkrail_plan_review::plan_ops::{self, ItemPatch, NewItem, WriteGroup, WriteItem};

use super::args::{field, optional_status, parse_status, string};
use super::render::{
    consistency_nudge, format_item, format_plan, next_open_step, plan_complete_nudge, status_name,
    with_nudges,
};
use super::{HostTools, Outcome, refuse};

/// Lines per `todo_list` page (pi's 2,000-line limit minus headroom) and its byte budget.
const PAGE_LINES: usize = 1990;
const PAGE_BYTES: usize = 50 * 1024 - 2048;

impl HostTools {
    pub(super) fn list(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let offset = match args.get("offset") {
            None | Some(Value::Null) => 0,
            Some(value) => match value.as_u64().and_then(|n| usize::try_from(n).ok()) {
                Some(n) => n,
                None => return Ok(Err(refuse("Use a nonnegative integer offset."))),
            },
        };
        let status = match optional_status(args)? {
            Ok(status) => status,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let plan = self.plans.plan(scope)?;
        let rendered = match status {
            Some(status) => {
                let lines: Vec<String> = plan_ops::items(&plan)
                    .filter(|i| i.status == status)
                    .map(format_item)
                    .collect();
                if lines.is_empty() {
                    format!("No {} TODOs.", status_name(status))
                } else {
                    lines.join("\n")
                }
            }
            None if plan_ops::items(&plan).next().is_none() && plan.groups.is_empty() => {
                "The plan is empty.".to_owned()
            }
            None => with_nudges(format_plan(&plan), &[consistency_nudge(&plan)]),
        };
        let lines: Vec<&str> = rendered.split('\n').collect();
        if offset >= lines.len() {
            return Ok(Ok(
                "No more plan lines. Restart at offset: 0 if the plan changed.".to_owned(),
            ));
        }
        let mut page = String::new();
        let mut count = 0;
        for line in lines.iter().skip(offset).take(PAGE_LINES) {
            if count > 0 && page.len() + line.len() + 1 > PAGE_BYTES {
                break;
            }
            if count > 0 {
                page.push('\n');
            }
            page.push_str(line);
            count += 1;
        }
        let next = offset + count;
        if next < lines.len() {
            let filter = status
                .map(|s| format!(" and status: {}", status_name(s)))
                .unwrap_or_default();
            page.push_str(&format!(
                "\n\nMore plan lines: call todo_list with offset: {next}{filter}. Restart at offset: 0 after plan edits."
            ));
        }
        Ok(Ok(page))
    }

    pub(super) fn add(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(title) = string(args, "title") else {
            return Ok(Err(refuse("`title` is required.")));
        };
        let after = string(args, "after").map(PlanItemId::from);
        if let Some(after) = &after {
            let plan = self.plans.plan(scope)?;
            if !plan
                .groups
                .iter()
                .any(|g| g.items.iter().any(|i| &i.id == after))
            {
                let loose = plan.loose.iter().any(|i| &i.id == after);
                return Ok(Err(refuse(if loose {
                    format!(
                        "\"{after}\" is a loose item — anchor to a step inside a task, or pass `group` to append there (loose items are promoted into a group, not grown with siblings)."
                    )
                } else {
                    format!("No step with id \"{after}\" to insert after.")
                })));
            }
        }
        let input = NewItem {
            title,
            origin: None,
            group: if after.is_none() {
                string(args, "group")
            } else {
                None
            },
            after,
            note: string(args, "note"),
        };
        Ok(self
            .mutate(scope, |plan| plan_ops::add(plan, input.clone()))?
            .map(|(plan, item)| {
                with_nudges(
                    format!("Added: {}", format_item(&item)),
                    &[consistency_nudge(&plan)],
                )
            }))
    }

    pub(super) fn update(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(id) = string(args, "id").map(PlanItemId::from) else {
            return Ok(Err(refuse("`id` is required.")));
        };
        let status = match optional_status(args)? {
            Ok(status) => status,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let patch = ItemPatch {
            title: string(args, "title"),
            status,
            note: field(args, "note"),
            summary: field(args, "summary"),
            verification: field(args, "verification"),
            commit_subject: field(args, "commitSubject"),
        };
        let updated = match self.mutate(scope, |plan| plan_ops::update(plan, &id, patch.clone()))? {
            Ok(updated) => updated,
            Err(_) => return Ok(Err(refuse(format!("No TODO with id \"{id}\".")))),
        };
        let (plan, result) = updated;
        let mut text = format!("Updated: {}", format_item(&result.item));
        if !result.paused.is_empty() {
            let paused: Vec<String> = result
                .paused
                .iter()
                .map(|p| {
                    format!(
                        "{p} \"{}\"",
                        plan_ops::find(&plan, p)
                            .map(|i| i.title.as_str())
                            .unwrap_or_default()
                    )
                })
                .collect();
            text.push_str(&format!(
                "\n(paused: {} — back to pending)",
                paused.join(", ")
            ));
        }
        let next = (status == Some(PlanStatus::Done))
            .then(|| next_open_step(&plan, &id))
            .flatten();
        let next_nudge = next.map(|n| {
            format!(
                "next: {} \"{}\" — mark it in_progress when you start.",
                n.id, n.title
            )
        });
        let consistency = if next_nudge.is_some() {
            None
        } else {
            consistency_nudge(&plan)
        };
        let complete = (status == Some(PlanStatus::Done))
            .then(|| plan_complete_nudge(&plan))
            .flatten();
        Ok(Ok(with_nudges(text, &[next_nudge, consistency, complete])))
    }

    pub(super) fn remove(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(id) = string(args, "id").map(PlanItemId::from) else {
            return Ok(Err(refuse("`id` is required.")));
        };
        Ok(
            match self.mutate(scope, |plan| plan_ops::remove(plan, &id))? {
                Ok(_) => Ok(format!("Removed {id}.")),
                Err(_) => Err(refuse(format!("No TODO with id \"{id}\"."))),
            },
        )
    }

    pub(super) fn promote(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(id) = string(args, "id").map(PlanItemId::from) else {
            return Ok(Err(refuse("`id` is required.")));
        };
        let title = string(args, "title");
        let before = self.plans.plan(scope)?;
        match self.mutate(scope, |plan| plan_ops::promote(plan, &id, title.clone()))? {
            Ok((plan, group)) => Ok(Ok(with_nudges(
                format!(
                    "Promoted {id} → group \"{}\" ({}). Add its first step with todo_add group:\"{}\" and flip it in_progress.",
                    group.title, group.id, group.title
                ),
                &[consistency_nudge(&plan)],
            ))),
            Err(_) => {
                let grouped = before
                    .groups
                    .iter()
                    .any(|g| g.items.iter().any(|i| i.id == id));
                let loose = before.loose.iter().any(|i| i.id == id);
                let shown = title
                    .as_deref()
                    .map(|t| format!("\"{t}\""))
                    .unwrap_or_else(|| "the loose item's title".into());
                Ok(Err(refuse(if grouped {
                    format!(
                        "\"{id}\" is already a step inside a group. Promotion only applies to loose items (todo_promote adds a group of its own; a grouped step is already in one)."
                    )
                } else if !loose {
                    format!(
                        "No TODO with id \"{id}\". Check the id with todo_list; a stale id from an older plan read never matches."
                    )
                } else if title.as_deref().is_some_and(|t| t.trim().is_empty()) {
                    "Resolved group title is empty. Pass a non-empty `title:` so the group has an addressable name for later todo_add group: calls.".to_owned()
                } else {
                    format!(
                        "A group titled {shown} already exists. Pass a disambiguating `title:` on todo_promote (a silent merge would make the loose ask disappear without adding any step)."
                    )
                })))
            }
        }
    }

    pub(super) fn write(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(raw) = args.get("groups").and_then(Value::as_array) else {
            return Ok(Err(refuse("`groups` must be an array.")));
        };
        let mut groups = Vec::with_capacity(raw.len());
        for group in raw {
            let Some(title) = string(group, "title") else {
                return Ok(Err(refuse("Each group needs a `title`.")));
            };
            let mut items = Vec::new();
            for todo in group
                .get("todos")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(step) = string(todo, "title") else {
                    return Ok(Err(refuse("Each todo needs a `title`.")));
                };
                let status = match todo.get("status").and_then(Value::as_str) {
                    None => None,
                    Some(raw) => match parse_status(raw) {
                        Some(status) => Some(status),
                        None => {
                            return Ok(Err(refuse(format!(
                                "Unknown status \"{raw}\"; use pending | in_progress | done."
                            ))));
                        }
                    },
                };
                items.push(WriteItem {
                    title: step,
                    status,
                    note: string(todo, "note"),
                });
            }
            groups.push(WriteGroup { title, items });
        }
        Ok(self
            .mutate(scope, |plan| plan_ops::write_groups(plan, groups.clone()))?
            .map(|(plan, ())| {
                let count = plan_ops::items(&plan).count();
                if count == 0 && plan.groups.is_empty() {
                    "Cleared the plan.".to_owned()
                } else {
                    with_nudges(
                        format!(
                            "Wrote the plan ({count} item(s) total):\n{}",
                            format_plan(&plan)
                        ),
                        &[consistency_nudge(&plan)],
                    )
                }
            }))
    }

    pub(super) fn summary(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(summary) = args.get("summary").and_then(Value::as_str) else {
            return Ok(Err(refuse("`summary` is required.")));
        };
        let summary = summary.trim().to_owned();
        Ok(self.mutate(scope, |plan| {
            plan_ops::set_summary(plan, &summary);
            Ok(())
        })?
        .map(|(plan, ())| {
            if summary.is_empty() {
                return "Plan summary cleared.".to_owned();
            }
            let open = plan_ops::items(&plan).filter(|i| i.status != PlanStatus::Done).count();
            let note = (open > 0).then(|| format!("note: {open} item(s) are still open — the summary shows once everything is done."));
            with_nudges("Plan summary saved.".to_owned(), &[note])
        }))
    }
}
