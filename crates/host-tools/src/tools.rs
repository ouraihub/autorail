//! The TODO / review tools (04 §6.1) plus the `spec_*` tools (`crate::spec`). Names, schemas and
//! model-facing wording are the original ThinkRail tools' (`specs/todo-tools.json` is exported from
//! `packages/pi-todos`); semantics come from `plan_ops`. A tool-level failure is a result with
//! `is_error`, never an `Err`.

use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use thinkrail_core::{
    Error, ErrorKind, HostToolHandler, HostToolOptions, HostToolResult, HostToolSpec,
    ModelPrefService, Plan, PlanReviewService, PlanScope, Result, ReviewRunner, WorkspaceRoots,
    WorkspaceService,
};

mod args;
mod plan;
mod render;
mod request;
mod review;
mod window;

const TODO_SPECS: &str = include_str!("../specs/todo-tools.json");
const RESOLVE_COMMENT: &str = "resolve_comment";
const REQUEST_REVIEW: &str = "request_review";
const RESOLVE_DESCRIPTION: &str = "Mark a review comment or a reviewer finding as resolved, after you have actually addressed it (by editing the file, or by answering when no change is needed). Only valid for comment ids you received in a review package, and finding ids (rf_…) you received in a review verdict, in this conversation. If a comment or finding is unclear or you disagree with it, reply in the conversation instead — do NOT resolve it.";

static SPECS: LazyLock<Vec<HostToolSpec>> = LazyLock::new(|| {
    let mut specs = bundled("TODO", TODO_SPECS);
    specs.extend(bundled("spec", crate::spec::SPEC_SPECS));
    specs.push(HostToolSpec {
        name: RESOLVE_COMMENT.into(),
        description: RESOLVE_DESCRIPTION.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "commentId": {"type": "string", "description": "The review comment id from the review package (e.g. \"rc_1a2b3c4d\") or the finding id from a review verdict (e.g. \"rf_…\")."},
                "note": {"type": "string", "description": "One short line: what you did about the comment (shown in the review sidebar)."}
            },
            "required": ["commentId"]
        }),
    });
    specs
});

/// `request_review` as the tool result carries the verdict, then as the verdict comes later.
static REVIEW_SPECS: LazyLock<Vec<HostToolSpec>> =
    LazyLock::new(|| bundled("review", include_str!("../specs/review-tools.json")));

fn bundled(what: &str, json: &str) -> Vec<HostToolSpec> {
    serde_json::from_str(json).unwrap_or_else(|error| {
        tracing::error!(%error, "bundled {what} tool specs are invalid");
        Vec::new()
    })
}

/// Dispatches host tool calls for one `(workspace, chat)` scope onto the plan/review store and,
/// for `spec_*`, the scope's worktree.
#[derive(Clone)]
pub struct HostTools {
    plans: Arc<dyn PlanReviewService>,
    roots: Arc<dyn WorkspaceRoots>,
    /// Git for work windows (08 §5.8); without it steps get no change set.
    workspaces: Option<Arc<dyn WorkspaceService>>,
    windows: Arc<window::WindowQueue>,
    /// `request_review` (08 §5.2); without it the tool is never listed.
    reviews: Option<request::Reviews>,
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
        Self {
            plans,
            roots,
            workspaces: None,
            windows: Arc::default(),
            reviews: None,
        }
    }

    /// Enables work windows: after each `todo_*` call the host attributes changes to steps and
    /// commits a finished step's delta.
    pub fn with_workspaces(mut self, workspaces: Arc<dyn WorkspaceService>) -> Self {
        self.workspaces = Some(workspaces);
        self
    }

    /// Enables `request_review`: `runner` runs the reviews, `prefs` is read at every call so the
    /// 评审 switch applies before the engine has restarted.
    pub fn with_reviews(
        mut self,
        runner: Arc<dyn ReviewRunner>,
        prefs: Arc<dyn ModelPrefService>,
    ) -> Self {
        self.reviews = Some(request::Reviews { runner, prefs });
        self
    }

    /// Waits for the workspace's running reconcile pass, so a change set read next is current.
    pub fn settle_windows(&self, scope: &PlanScope) {
        self.windows.settle(&scope.workspace);
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
    fn specs(&self, options: HostToolOptions) -> Vec<HostToolSpec> {
        let mut specs = SPECS.clone();
        if options.request_review && self.reviews.is_some() {
            // `review-tools.json` holds the waiting (pi/OMP) and the message (DSH) variant.
            let variant = usize::from(options.async_review);
            specs.extend(REVIEW_SPECS.get(variant).cloned());
        }
        specs
    }

    fn cancel(&self, scope: &PlanScope) {
        if let Some(reviews) = &self.reviews {
            reviews.runner.cancel(scope);
        }
    }

    fn call(
        &self,
        scope: &PlanScope,
        options: HostToolOptions,
        name: &str,
        arguments: Value,
    ) -> Result<HostToolResult> {
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
            REQUEST_REVIEW => self.request_review(scope, options, &args)?,
            other => Err(refuse(format!("Unknown host tool \"{other}\"."))),
        };
        // The original reconciles after every `todo_*` tool, refused calls included.
        if name.starts_with("todo_")
            && let Some(workspaces) = &self.workspaces
        {
            self.windows
                .reconcile(self.plans.as_ref(), workspaces.as_ref(), scope);
        }
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
