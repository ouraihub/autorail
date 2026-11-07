//! `resolve_comment`: marking a review comment or a reviewer finding the model received as
//! resolved.

use serde_json::Value;
use thinkrail_core::{PlanScope, Result, ReviewId, ReviewState};
use thinkrail_plan_review::verdict;

use super::args::string;
use super::{HostTools, Outcome, classify, refuse};

impl HostTools {
    pub(super) fn resolve(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
        let Some(id) = string(args, "commentId") else {
            return Ok(Err(refuse("`commentId` is required.")));
        };
        if id.starts_with("rf_") {
            return self.resolve_finding(scope, &id);
        }
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

    /// Reviewer findings live with the step's verdict; every open one was given to the worker.
    fn resolve_finding(&self, scope: &PlanScope, id: &str) -> Result<Outcome> {
        let mut resolved = false;
        self.plans.update_reviews(scope, &mut |_, steps| {
            resolved = verdict::resolve_finding(steps, id);
            Ok(())
        })?;
        Ok(if resolved {
            Ok(format!("Resolved reviewer finding {id}."))
        } else {
            Err(refuse(format!(
                "No open reviewer finding with id \"{id}\" in this conversation."
            )))
        })
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.to_owned()
    }
}
