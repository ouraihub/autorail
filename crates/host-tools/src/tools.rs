//! The TODO / review tools (04 §6.1) plus the `spec_*` tools (`crate::spec`). Names, schemas and
//! model-facing wording are the original ThinkRail tools' (`specs/todo-tools.json` is exported from
//! `packages/pi-todos`); semantics come from `plan_ops`. A tool-level failure is a result with
//! `is_error`, never an `Err`.

use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use thinkrail_core::{
    Error, ErrorKind, HostToolHandler, HostToolResult, HostToolSpec, Plan, PlanItem, PlanItemId,
    PlanReviewService, PlanScope, PlanStatus, Result, ReviewId, ReviewState, WorkspaceRoots,
};
use thinkrail_plan_review::plan_ops::{
    self, GroupStatus, ItemPatch, NewItem, WriteGroup, WriteItem,
};

const TODO_SPECS: &str = include_str!("../specs/todo-tools.json");
/// Lines per `todo_list` page (pi's 2,000-line limit minus headroom) and its byte budget.
const PAGE_LINES: usize = 1990;
const PAGE_BYTES: usize = 50 * 1024 - 2048;
const RESOLVE_COMMENT: &str = "resolve_comment";
const RESOLVE_DESCRIPTION: &str = "Mark a review comment as resolved, after you have actually addressed it (by editing the file, or by answering when no change is needed). Only valid for comment ids you received in a review package in this conversation. If a comment is unclear or you disagree with it, reply in the conversation instead — do NOT resolve it.";

static SPECS: LazyLock<Vec<HostToolSpec>> = LazyLock::new(|| {
    let mut specs: Vec<HostToolSpec> = Vec::new();
    for (what, json) in [("TODO", TODO_SPECS), ("spec", crate::spec::SPEC_SPECS)] {
        match serde_json::from_str::<Vec<HostToolSpec>>(json) {
            Ok(parsed) => specs.extend(parsed),
            Err(error) => tracing::error!(%error, "bundled {what} tool specs are invalid"),
        }
    }
    specs.push(HostToolSpec {
        name: RESOLVE_COMMENT.into(),
        description: RESOLVE_DESCRIPTION.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "commentId": {"type": "string", "description": "The review comment id from the review package (e.g. \"rc_1a2b3c4d\")."},
                "note": {"type": "string", "description": "One short line: what you did about the comment (shown in the review sidebar)."}
            },
            "required": ["commentId"]
        }),
    });
    specs
});

/// Dispatches host tool calls for one `(workspace, chat)` scope onto the plan/review store and,
/// for `spec_*`, the scope's worktree.
#[derive(Clone)]
pub struct HostTools {
    plans: Arc<dyn PlanReviewService>,
    roots: Arc<dyn WorkspaceRoots>,
}

impl std::fmt::Debug for HostTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostTools").finish_non_exhaustive()
    }
}

/// A tool-level failure the model should see.
struct Refusal(String);

type Outcome = std::result::Result<String, Refusal>;

fn refuse(text: impl Into<String>) -> Refusal {
    Refusal(text.into())
}

/// Domain errors become tool-level refusals; only infrastructure failures stay `Err`.
fn classify(error: Error) -> Result<Refusal> {
    match error.kind() {
        Some(
            ErrorKind::NotFound | ErrorKind::Invalid | ErrorKind::Conflict | ErrorKind::Forbidden,
        ) => Ok(Refusal(error.to_string())),
        _ => Err(error),
    }
}

impl HostTools {
    pub fn new(plans: Arc<dyn PlanReviewService>, roots: Arc<dyn WorkspaceRoots>) -> Self {
        Self { plans, roots }
    }

    /// Runs one plan mutation; domain failures surface as refusals.
    fn mutate<T>(
        &self,
        scope: &PlanScope,
        mut change: impl FnMut(&mut Plan) -> Result<T>,
    ) -> Result<std::result::Result<(Plan, T), Refusal>> {
        let mut out = None;
        let result = self.plans.update_plan(scope, &mut |plan| {
            out = Some(change(plan)?);
            Ok(())
        });
        match (result, out) {
            (Ok(plan), Some(value)) => Ok(Ok((plan, value))),
            (Ok(_), None) => Err(Error::corrupt("计划更新没有返回结果")),
            (Err(error), _) => classify(error).map(Err),
        }
    }

    fn list(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn add(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn update(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn remove(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn promote(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn write(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn summary(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

    fn resolve(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(id) = string(args, "commentId") else {
            return Ok(Err(refuse("`commentId` is required.")));
        };
        let id = ReviewId::from(id);
        let comment = self.plans.reviews(scope)?.into_iter().find(|c| c.id == id);
        match comment {
            Some(c) if c.state == ReviewState::Sent => {}
            Some(c) if c.state == ReviewState::Resolved => {
                return Ok(Err(refuse(format!(
                    "Review comment {id} is already resolved."
                ))));
            }
            Some(_) => {
                return Ok(Err(refuse(format!(
                    "Review comment {id} has not been sent to you in a review package yet; only comments you received can be resolved."
                ))));
            }
            None => {
                return Ok(Err(refuse(format!(
                    "No review comment with id \"{id}\" in this conversation."
                ))));
            }
        }
        match self.plans.resolve_review(scope, &id) {
            Ok(resolved) => Ok(Ok(format!(
                "Resolved review comment {id} (\"{}\").",
                truncate(&resolved.body, 80)
            ))),
            Err(error) => classify(error).map(Err),
        }
    }
}

impl HostToolHandler for HostTools {
    fn specs(&self) -> Vec<HostToolSpec> {
        SPECS.clone()
    }

    fn call(&self, scope: &PlanScope, name: &str, arguments: Value) -> Result<HostToolResult> {
        let args = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        if !args.is_object() {
            return Ok(HostToolResult {
                text: "Tool arguments must be a JSON object.".into(),
                is_error: true,
                structured: None,
            });
        }
        if crate::spec::is_spec_tool(name) {
            return match self.roots.workspace_root(&scope.workspace) {
                Ok(root) => Ok(crate::spec::call(&root, name, &args)),
                Err(error) => {
                    let Refusal(text) = classify(error)?;
                    Ok(HostToolResult {
                        text,
                        is_error: true,
                        structured: None,
                    })
                }
            };
        }
        let outcome = match name {
            "todo_list" => self.list(scope, &args)?,
            "todo_add" => self.add(scope, &args)?,
            "todo_update" => self.update(scope, &args)?,
            "todo_remove" => self.remove(scope, &args)?,
            "todo_promote" => self.promote(scope, &args)?,
            "todo_write" => self.write(scope, &args)?,
            "todo_plan_summary" => self.summary(scope, &args)?,
            RESOLVE_COMMENT => self.resolve(scope, &args)?,
            other => Err(refuse(format!("Unknown host tool \"{other}\"."))),
        };
        Ok(match outcome {
            Ok(text) => HostToolResult {
                text,
                is_error: false,
                structured: None,
            },
            Err(Refusal(text)) => HostToolResult {
                text,
                is_error: true,
                structured: None,
            },
        })
    }
}

fn string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A string field where `""` clears: absent → untouched, present → set or cleared.
fn field(args: &Value, key: &str) -> Option<Option<String>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| (!s.trim().is_empty()).then(|| s.to_owned()))
}

fn parse_status(raw: &str) -> Option<PlanStatus> {
    match raw {
        "pending" => Some(PlanStatus::Pending),
        "in_progress" => Some(PlanStatus::InProgress),
        "done" => Some(PlanStatus::Done),
        _ => None,
    }
}

fn optional_status(args: &Value) -> Result<std::result::Result<Option<PlanStatus>, Refusal>> {
    Ok(match args.get("status").and_then(Value::as_str) {
        None => Ok(None),
        Some(raw) => parse_status(raw).map(Some).ok_or_else(|| {
            refuse(format!(
                "Unknown status \"{raw}\"; use pending | in_progress | done."
            ))
        }),
    })
}

fn status_name(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Pending => "pending",
        PlanStatus::InProgress => "in_progress",
        PlanStatus::Done => "done",
    }
}

fn format_item(item: &PlanItem) -> String {
    let glyph = match item.status {
        PlanStatus::Pending => "[ ]",
        PlanStatus::InProgress => "[~]",
        PlanStatus::Done => "[x]",
    };
    format!("{glyph} {} — {}", item.title, item.id)
}

fn format_plan(plan: &Plan) -> String {
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

fn consistency_nudge(plan: &Plan) -> Option<String> {
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

fn next_open_step<'a>(plan: &'a Plan, id: &PlanItemId) -> Option<&'a PlanItem> {
    let group = plan
        .groups
        .iter()
        .find(|g| g.items.iter().any(|i| &i.id == id))?;
    group.items.iter().find(|i| i.status != PlanStatus::Done)
}

fn plan_complete_nudge(plan: &Plan) -> Option<String> {
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

fn with_nudges(text: String, nudges: &[Option<String>]) -> String {
    let extra: Vec<&str> = nudges.iter().flatten().map(String::as_str).collect();
    if extra.is_empty() {
        text
    } else {
        format!("{text}\n{}", extra.join("\n"))
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.to_owned()
    }
}
