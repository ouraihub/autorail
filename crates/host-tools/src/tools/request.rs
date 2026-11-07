//! `request_review` (agent-settings 08 §5.2, §5.5): checks the step, then starts a plan-step
//! review and, on pi/OMP, waits for its verdict; on DSH the verdict comes back as a message.

use std::sync::Arc;

use serde_json::Value;
use thinkrail_core::{
    HostToolOptions, ModelPrefService, PlanItemId, PlanScope, PlanStatus, Result, ReviewOrigin,
    ReviewOutcome, ReviewRunner,
};
use thinkrail_plan_review::plan_ops;

use super::args::string;
use super::{HostTools, Outcome, classify, refuse};

/// What `request_review` needs from the app: the runner and the live 评审 settings.
#[derive(Clone)]
pub(super) struct Reviews {
    pub(super) runner: Arc<dyn ReviewRunner>,
    pub(super) prefs: Arc<dyn ModelPrefService>,
}

const OFF: &str = "Agent-triggered review is turned off. Do not request a review; report the step as done instead.";
const STARTED: &str = "The review has started. Its verdict will come to you as a new message starting with 【评审结论】; until then, go on with the next step and do not modify this step's files.";
const RUNNING: &str = "A review of this step is already running. Its verdict will come to you as a new message starting with 【评审结论】; until then, go on with the next step and do not modify this step's files.";

impl HostTools {
    pub(super) fn request_review(
        &self,
        scope: &PlanScope,
        options: HostToolOptions,
        args: &Value,
    ) -> Result<Outcome> {
        let Some(reviews) = self.reviews.as_ref().filter(|_| options.request_review) else {
            return Ok(Err(refuse(
                "Plan-step review is not available in this chat.",
            )));
        };
        // The engine restarts when the switch changes; until it has, the setting wins.
        if !reviews.prefs.prefs()?.review.agent_review {
            return Ok(Err(refuse(OFF)));
        }
        let Some(id) = string(args, "itemId") else {
            return Ok(Err(refuse("`itemId` is required.")));
        };
        let id = PlanItemId::from(id);
        // The `todo_*` call that marked the step done may still be attributing its changes.
        self.settle_windows(scope);
        let plan = self.plans.plan(scope)?;
        let Some(item) = plan_ops::items(&plan).find(|item| item.id == id) else {
            return Ok(Err(refuse(format!("No plan step with id \"{id}\"."))));
        };
        if item.status != PlanStatus::Done {
            return Ok(Err(refuse(format!(
                "Step \"{}\" is not done; mark it done before requesting a review.",
                item.title
            ))));
        }
        if item.changes.is_empty() {
            return Ok(Err(refuse(format!(
                "Step \"{}\" has no reviewable changes: the host found no commits or changed files in its work window.",
                item.title
            ))));
        }
        let waits = !options.async_review;
        let started = match reviews
            .runner
            .start(scope, &id, ReviewOrigin::Tool { waits })
        {
            Ok(started) => started,
            Err(error) => return classify(error).map(Err),
        };
        if !waits {
            return Ok(Ok(if started.joined { RUNNING } else { STARTED }.to_owned()));
        }
        let result = started.handle.wait();
        Ok(match result.outcome {
            ReviewOutcome::Failed { .. } => Err(refuse(result.text)),
            _ => Ok(result.text),
        })
    }
}
