//! `resolve_comment`: marking a review comment the model received as resolved.

use serde_json::Value;
use thinkrail_core::{PlanScope, Result, ReviewId, ReviewState};

use super::args::string;
use super::{HostTools, Outcome, classify, refuse};

impl HostTools {
    pub(super) fn resolve(&self, scope: &PlanScope, args: &Value) -> Result<Outcome> {
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

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    } else {
        text.to_owned()
    }
}
