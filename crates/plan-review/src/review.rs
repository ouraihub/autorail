//! Text a plan-step review exchanges with the models (agent-settings 08 §5.4): the reviewer's
//! system prompt and fact package, what the worker reads after a verdict, and the fix request the
//! user sends. Model-facing text is English and lives in `prompts/`, copied from the original.

use thinkrail_core::{ItemChange, PlanItem, ReviewFinding, ReviewOutcome, ReviewRecord};

pub const REVIEWER_SYSTEM: &str = include_str!("../prompts/reviewer_system.md");
const PACKAGE: &str = include_str!("../prompts/review_package.md");
const RESULT: &str = include_str!("../prompts/tool_result.md");
const FIX: &str = include_str!("../prompts/fix_package.md");

/// The `## key` section of a prompt file with each `{name}` replaced in one pass, so a value that
/// itself contains `{…}` (a step title, a finding) is never substituted again. Files may be checked
/// out with CRLF.
fn section(file: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let mut lines = file.lines().map(|line| line.trim_end_matches('\r'));
    let heading = format!("## {key}");
    lines
        .by_ref()
        .find(|line| *line == heading)
        .unwrap_or_else(|| panic!("prompt section `{key}` is missing"));
    let template = lines
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(template.len());
    let mut rest = template.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let value = tail.find('}').and_then(|close| {
            let name = &tail[1..close];
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &tail[close + 1..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The first 12 characters, as the original prints shas.
fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// What is reviewed: a plan step, or a commit no step owns (08 §5.8).
#[derive(Debug, Clone, Copy)]
pub enum Subject<'a> {
    Step(&'a PlanItem),
    Commit { sha: &'a str, title: &'a str },
}

impl Subject<'_> {
    pub fn title(&self) -> &str {
        match self {
            Subject::Step(item) => &item.title,
            Subject::Commit { title, .. } => title,
        }
    }

    fn id(&self) -> &str {
        match self {
            Subject::Step(item) => item.id.as_str(),
            Subject::Commit { sha, .. } => short(sha),
        }
    }

    /// The commits the subject owns, oldest first.
    pub fn shas(&self) -> Vec<&str> {
        match self {
            Subject::Step(item) => item
                .changes
                .iter()
                .filter_map(|change| match change {
                    ItemChange::Commit { sha, .. } => Some(sha.as_str()),
                    ItemChange::Path { .. } => None,
                })
                .collect(),
            Subject::Commit { sha, .. } => vec![sha],
        }
    }

    fn paths(&self) -> Vec<&str> {
        match self {
            Subject::Step(item) => item
                .changes
                .iter()
                .filter_map(|change| match change {
                    ItemChange::Path { path } => Some(path.as_str()),
                    ItemChange::Commit { .. } => None,
                })
                .collect(),
            Subject::Commit { .. } => Vec::new(),
        }
    }

    /// "commit(s) …; uncommitted paths: …" or "changed paths: …"; `None` when there is nothing to
    /// review.
    pub fn change_set(&self) -> Option<String> {
        let (shas, paths) = (self.shas(), self.paths());
        if shas.is_empty() && paths.is_empty() {
            return None;
        }
        if shas.is_empty() {
            return Some(format!("changed paths: {}", paths.join(", ")));
        }
        let plural = if shas.len() == 1 { "" } else { "s" };
        let list = shas
            .iter()
            .map(|sha| short(sha))
            .collect::<Vec<_>>()
            .join(", ");
        let uncommitted = if paths.is_empty() {
            String::new()
        } else {
            format!("; uncommitted paths: {}", paths.join(", "))
        };
        Some(format!("commit{plural} {list}{uncommitted}"))
    }

    fn note(&self) -> Option<&str> {
        match self {
            Subject::Step(item) => item.note.as_deref(),
            Subject::Commit { .. } => None,
        }
    }

    fn summary(&self) -> Option<&str> {
        match self {
            Subject::Step(item) => item.summary.as_deref(),
            Subject::Commit { .. } => None,
        }
    }

    fn verification(&self) -> Option<&str> {
        match self {
            Subject::Step(item) => item.verification.as_deref(),
            Subject::Commit { .. } => None,
        }
    }
}

/// The reviewer's first message: the facts, a RE-REVIEW delta when only some commits are new
/// since `prior`, and the output contract. `None` when the subject has no change set.
pub fn review_package(
    subject: Subject<'_>,
    chat: &str,
    prior: Option<&ReviewRecord>,
) -> Option<String> {
    let change_set = subject.change_set()?;
    let base = [
        ("id", subject.id()),
        ("title", subject.title()),
        ("chat", chat),
    ];
    let line = |key: &str, extra: (&str, &str)| {
        let mut vars = base.to_vec();
        vars.push(extra);
        section(PACKAGE, key, &vars)
    };
    let mut lines = vec![match subject {
        Subject::Step(_) => section(PACKAGE, "step", &base),
        Subject::Commit { sha, .. } => line("commit", ("sha", short(sha))),
    }];
    lines.push(String::new());
    if let Some(note) = subject.note() {
        lines.push(line("note", ("note", note)));
    }
    if let Some(summary) = subject.summary() {
        lines.push(line("summary", ("summary", summary)));
    }
    lines.push(match subject.verification() {
        Some(claim) => line("verification", ("verification", claim)),
        None => section(PACKAGE, "no_verification", &[]),
    });
    lines.push(line("change_set", ("change_set", &change_set)));
    let shas = subject.shas();
    if let Some(prior) = prior {
        let fresh: Vec<&str> = shas
            .iter()
            .copied()
            .filter(|sha| !prior.reviewed_shas.iter().any(|seen| seen == sha))
            .map(short)
            .collect();
        if !fresh.is_empty() && fresh.len() < shas.len() {
            lines.push(line("rereview", ("fresh", &fresh.join(", "))));
        }
    }
    let contract = section(PACKAGE, "contract", &[]);
    Some(format!("{}\n\n{contract}", lines.join("\n")))
}

/// `- [rf_…] (path:start-end) body`, one line per finding.
fn finding_lines(findings: &[ReviewFinding]) -> String {
    findings
        .iter()
        .map(|finding| {
            let location = match (&finding.path, finding.start_line, finding.end_line) {
                (Some(path), Some(start), Some(end)) if end != start => {
                    format!(" ({path}:{start}-{end})")
                }
                (Some(path), Some(start), _) => format!(" ({path}:{start})"),
                (Some(path), None, _) => format!(" ({path})"),
                (None, ..) => String::new(),
            };
            format!("- [{}]{location} {}", finding.id, finding.body)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the worker reads after a review: the verdict, the next action, the reviewer's summary and
/// `findings` (the new ones for a request for changes, the reviewer's notes for an approval).
pub fn result_text(
    title: &str,
    outcome: &ReviewOutcome,
    summary: Option<&str>,
    findings: &[ReviewFinding],
) -> String {
    let title = [("title", title)];
    let fill = |key: &str| section(RESULT, key, &title);
    let head = match outcome {
        ReviewOutcome::Failed { message } => {
            return section(RESULT, "failed", &[title[0], ("message", message)]);
        }
        ReviewOutcome::Approved => fill("approved"),
        ReviewOutcome::ApproveBlocked { open } => section(
            RESULT,
            "approve_blocked",
            &[title[0], ("open", &open.to_string())],
        ),
        ReviewOutcome::ChangesFix => format!("{} {}", fill("changes"), fill("fix_now")),
        ReviewOutcome::ChangesStop => format!("{} {}", fill("changes"), fill("stop")),
    };
    let mut text = head;
    if let Some(summary) = summary.filter(|s| !s.is_empty()) {
        text.push_str("\n\n");
        text.push_str(summary);
    }
    if !findings.is_empty() {
        text.push_str("\n\n");
        text.push_str(&finding_lines(findings));
    }
    text
}

/// The user's 发给 Agent 修复: the step's facts and its open findings as the feedback. `None` when
/// the subject has no change set.
pub fn fix_package(subject: Subject<'_>, findings: &[ReviewFinding]) -> Option<String> {
    let change_set = subject.change_set()?;
    let base = [
        ("id", subject.id()),
        ("title", subject.title()),
        ("change_set", change_set.as_str()),
    ];
    let fill = |key: &str| section(FIX, key, &base);
    let line = |key: &str, extra: (&str, &str)| {
        let mut vars = base.to_vec();
        vars.push(extra);
        section(FIX, key, &vars)
    };
    let mut lines = vec![fill("head"), String::new()];
    if let Some(note) = subject.note() {
        lines.push(line("note", ("note", note)));
    }
    if let Some(summary) = subject.summary() {
        lines.push(line("summary", ("summary", summary)));
    }
    if let Some(claim) = subject.verification() {
        lines.push(line("verification", ("verification", claim)));
    }
    lines.push(fill("change_set"));
    lines.push(String::new());
    lines.push(fill("feedback"));
    lines.push("\"\"\"".into());
    lines.push(finding_lines(findings));
    lines.push("\"\"\"".into());
    lines.push(String::new());
    lines.push(match subject {
        Subject::Step(_) => fill("step"),
        Subject::Commit { .. } => fill("commit"),
    });
    Some(lines.join("\n"))
}
