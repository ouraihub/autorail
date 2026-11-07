//! The reviewer's verdict (agent-settings 08 §5.4, §5.6): parsing the model's reply, what a
//! verdict does to a step's review state, and when the host re-reviews on its own.

use serde_json::{Map, Value};
use thinkrail_core::{
    FindingState, ItemChange, ModelRef, PlanItem, PlanItemId, PlanStatus, ReviewFinding,
    ReviewOutcome, ReviewRecord, StepReviews, Verdict,
};

const READING_ORDER_LIMIT: usize = 12;
const KINDS: [&str; 4] = ["inline", "diff", "file", "review"];

/// A validated verdict. The reviewer's finding ids are dropped: the host assigns `rf_…` ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub verdict: Verdict,
    pub summary: Option<String>,
    pub findings: Vec<NewFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFinding {
    pub path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub body: String,
}

/// The first ```json block of `text`, else all of it, as a verdict. `None` when it is not valid
/// JSON or breaks the contract; such a reply is a failed review and is never retried. Unknown
/// fields are ignored, as in the original.
pub fn parse(text: &str) -> Option<Parsed> {
    let raw = fenced_json(text).unwrap_or(text);
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let object = value.as_object()?;
    let verdict = match object.get("verdict")?.as_str()? {
        "approve" => Verdict::Approve,
        "request_changes" => Verdict::RequestChanges,
        _ => return None,
    };
    let summary = match object.get("summary") {
        None => None,
        Some(value) => Some(value.as_str()?.to_owned()),
    };
    if let Some(order) = object.get("readingOrder") {
        let steps = order.as_array()?;
        let valid = steps.len() <= READING_ORDER_LIMIT
            && steps.iter().all(|step| {
                ["path", "why"].iter().all(|key| {
                    step.get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                })
            });
        if !valid {
            return None;
        }
    }
    let findings = object
        .get("findings")?
        .as_array()?
        .iter()
        .map(|finding| finding.as_object().and_then(new_finding))
        .collect::<Option<Vec<_>>>()?;
    if verdict == Verdict::RequestChanges && findings.is_empty() {
        return None;
    }
    Some(Parsed {
        verdict,
        summary,
        findings,
    })
}

/// The body of the first ```json fence (case-insensitive tag), up to the next fence.
fn fenced_json(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    let start = lower.find("```json")? + "```json".len();
    let end = text[start..].find("```")?;
    Some(&text[start..start + end])
}

fn positive(value: Option<&Value>) -> Option<Option<u32>> {
    match value {
        None => Some(None),
        Some(value) => {
            let n = value.as_u64().filter(|n| *n > 0)?;
            Some(Some(u32::try_from(n).ok()?))
        }
    }
}

/// Id and body required; a line needs a path, an end line a start line, ranges are positive and
/// ordered (the original `isReviewFinding`).
fn new_finding(f: &Map<String, Value>) -> Option<NewFinding> {
    f.get("id")?.as_str().filter(|id| !id.is_empty())?;
    let body = f
        .get("body")?
        .as_str()
        .filter(|b| !b.is_empty())?
        .to_owned();
    if let Some(kind) = f.get("kind") {
        KINDS.contains(&kind.as_str()?).then_some(())?;
    }
    let path = match f.get("path") {
        None => None,
        Some(path) => Some(path.as_str()?.replace('\\', "/")),
    };
    let start_line = positive(f.get("startLine"))?;
    let end_line = positive(f.get("endLine"))?;
    if (start_line.is_some() && path.is_none()) || (end_line.is_some() && start_line.is_none()) {
        return None;
    }
    if let (Some(start), Some(end)) = (start_line, end_line)
        && end < start
    {
        return None;
    }
    Some(NewFinding {
        path,
        start_line,
        end_line,
        body,
    })
}

fn open_count(record: Option<&ReviewRecord>) -> usize {
    record.map_or(0, |record| {
        record
            .findings
            .iter()
            .filter(|f| f.state == FindingState::Open)
            .count()
    })
}

/// What a verdict did, and the findings the worker is told about (the new ones).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub outcome: ReviewOutcome,
    pub record: Option<ReviewRecord>,
    pub findings: Vec<ReviewFinding>,
}

/// Applies a verdict on `item` (the §5.6 table) and ends its pending review. The watermark is the
/// commits as they were when the review started, so commits landed meanwhile stay unreviewed.
pub fn apply(
    steps: &mut StepReviews,
    item: &PlanItemId,
    parsed: Parsed,
    auto_fix: bool,
    model: Option<ModelRef>,
    now: u64,
) -> Applied {
    let reviewed_shas = steps.pending.remove(item).unwrap_or_default();
    let prior = steps.verdicts.get(item);
    let state = match parsed.verdict {
        // An approval's findings are notes, not blockers.
        Verdict::Approve => FindingState::Resolved,
        Verdict::RequestChanges => FindingState::Open,
    };
    let findings: Vec<ReviewFinding> = parsed
        .findings
        .into_iter()
        .map(|f| ReviewFinding {
            id: format!("rf_{}", uuid::Uuid::new_v4().simple()),
            path: f.path,
            start_line: f.start_line,
            end_line: f.end_line,
            body: f.body,
            state,
        })
        .collect();
    let outcome = match parsed.verdict {
        Verdict::Approve => {
            let open = open_count(prior);
            if open > 0 {
                return Applied {
                    outcome: ReviewOutcome::ApproveBlocked { open },
                    record: prior.cloned(),
                    findings,
                };
            }
            steps.auto_cycles.remove(item);
            ReviewOutcome::Approved
        }
        Verdict::RequestChanges => {
            let spent = steps.auto_cycles.get(item).copied().unwrap_or(0);
            let fix = auto_fix && spent == 0;
            steps
                .auto_cycles
                .insert(item.clone(), if fix { 1 } else { 2 });
            if fix {
                ReviewOutcome::ChangesFix
            } else {
                ReviewOutcome::ChangesStop
            }
        }
    };
    // Earlier findings the worker has not resolved still count on a re-review.
    let mut kept: Vec<ReviewFinding> = prior
        .map(|record| {
            record
                .findings
                .iter()
                .filter(|f| f.state == FindingState::Open)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    kept.extend(findings.iter().cloned());
    let record = ReviewRecord {
        verdict: parsed.verdict,
        summary: parsed.summary,
        findings: kept,
        reviewed_shas,
        model,
        at: now,
    };
    steps.verdicts.insert(item.clone(), record.clone());
    Applied {
        outcome,
        record: Some(record),
        findings,
    }
}

/// A failed review leaves the record alone and only ends the pending mark.
pub fn fail(steps: &mut StepReviews, item: &PlanItemId) {
    steps.pending.remove(item);
}

/// Resolves one open finding of this chat. When it was the last open one on a request for
/// changes, the step goes back to unreviewed (the original `clearChangesRequestedIfResolved`);
/// the spent fix round is kept. `false` when no open finding has that id.
pub fn resolve_finding(steps: &mut StepReviews, id: &str) -> bool {
    let found = steps.verdicts.iter_mut().find_map(|(item, record)| {
        let finding = record
            .findings
            .iter_mut()
            .find(|f| f.id == id && f.state == FindingState::Open)?;
        finding.state = FindingState::Resolved;
        let settled = record.verdict == Verdict::RequestChanges && open_count(Some(record)) == 0;
        Some(settled.then(|| item.clone()))
    });
    match found {
        Some(Some(item)) => {
            steps.verdicts.remove(&item);
            true
        }
        Some(None) => true,
        None => false,
    }
}

/// The host re-reviews a fixed step itself (the original `maybeAutoReReview`): its one fix round
/// is under way, it is done again, no review is running, and it has commits the last verdict did
/// not see (or only paths, whose record was dropped).
pub fn needs_auto_rereview(steps: &StepReviews, item: &PlanItem) -> bool {
    if steps.auto_cycles.get(&item.id) != Some(&1)
        || item.status != PlanStatus::Done
        || steps.pending.contains_key(&item.id)
        || item.changes.is_empty()
    {
        return false;
    }
    let Some(record) = steps.verdicts.get(&item.id) else {
        return true;
    };
    item.changes.iter().any(|change| match change {
        ItemChange::Commit { sha, .. } => !record.reviewed_shas.contains(sha),
        ItemChange::Path { .. } => false,
    })
}
