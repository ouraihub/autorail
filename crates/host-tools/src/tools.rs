//! The TODO / review tools (04 §6.1) plus the `spec_*` tools (`crate::spec`). Names, schemas and
//! model-facing wording are the original ThinkRail tools' (`specs/todo-tools.json` is exported from
//! `packages/pi-todos`); semantics come from `plan_ops`. A tool-level failure is a result with
//! `is_error`, never an `Err`.

use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use thinkrail_core::{
    Error, ErrorKind, HostToolHandler, HostToolResult, HostToolSpec, Plan, PlanReviewService,
    PlanScope, Result, WorkspaceRoots,
};

mod args;
mod plan;
mod render;
mod review;

const TODO_SPECS: &str = include_str!("../specs/todo-tools.json");
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
