//! ThinkRail's workflow system (original `pi-thinkrail-workflow`, `spec-graph` and `pi-todos`
//! skills at thinkrail `9360d9fa`): the bundled skills every engine loads from a staged directory,
//! and the two system-prompt rules the original extensions inject (`WORKFLOW_RULE`, `SPEC_RULE`).
//! The `spec_*` and `todo_*` tools the skills call are host tools (`thinkrail-host-tools`).

use std::path::{Path, PathBuf};

use thinkrail_core::{BackendKind, Result};

use crate::host;

/// Staged under the launch's support directory; each skill is `<name>/SKILL.md` plus its references.
pub(crate) const SKILLS_DIR: &str = "skills";

macro_rules! bundled {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../skills/", $path)))),*]
    };
}

const SKILLS: &[(&str, &str)] = bundled![
    "asking-user-questions/SKILL.md",
    "brainstorming/SKILL.md",
    "brainstorming/amending-specs.md",
    "brainstorming/fixing-a-bug.md",
    "choosing-a-workflow/SKILL.md",
    "importing-a-codebase/SKILL.md",
    "setting-up-a-project/SKILL.md",
    "shipping-a-pr/SKILL.md",
    "shipping-a-pr/body.md",
    "shipping-a-pr/checks.md",
    "shipping-a-pr/creating.md",
    "shipping-a-pr/review-comments.md",
    "shipping-a-pr/screenshots.md",
    "shipping-a-pr/syncing.md",
    "spec-graph/SKILL.md",
    "starting-a-new-project/SKILL.md",
    "todos/SKILL.md",
    "writing-specs/SKILL.md",
    "writing-workflow-skills/SKILL.md",
];

/// Original `pi-thinkrail-workflow/index.ts` `WORKFLOW_RULE`.
pub(crate) const WORKFLOW_RULE: &str = "At the start of a new piece of work, read the choosing-a-workflow skill before any other tool call for project onboarding, PR lifecycle work (including checking a PR's CI), or any change to what the product does — a feature, a behavior change, or a bug fix, however small.\nNon-PR questions, explanations, and checks, and edits that change no behavior (typos, documentation or configuration only, mechanical refactors), proceed directly without loading or announcing a workflow.\nContinue work already routed to a workflow without routing it again; if that workflow's skill text is no longer in your context, read its SKILL.md again before the next step.";

/// Original `spec-graph/index.ts` `SPEC_RULE`.
pub(crate) const SPEC_RULE: &str = "Specs are this project's ground truth.\n- Consult the relevant spec when work is governed by or may alter a documented boundary, contract, invariant, behavior, or architecture decision; localized work need not read unrelated specs.\n- Treat applicable decisions and contracts as authoritative; reconcile contradictions instead of diverging.\n- When a change alters a boundary, contract, invariant, or decision, update the owning spec as part of that change.";

/// Section names as the original registers them (`systemPromptOptions.sections[...]`).
pub(crate) const RULES: [(&str, &str); 2] = [
    ("pi-thinkrail-workflow", WORKFLOW_RULE),
    ("pi-spec-graph", SPEC_RULE),
];

/// Writes the bundled skills under `support_dir/skills` (unchanged files are left alone) and
/// returns that directory for the engine's skill path.
pub(crate) fn stage_skills(support_dir: &Path) -> Result<PathBuf> {
    let root = support_dir.join(SKILLS_DIR);
    for (path, content) in SKILLS {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        host::write_support(&root.join(dir), name, content)?;
    }
    Ok(crate::process::plain_path(&root))
}

/// How the user invokes skill `name` in a prompt: pi and OMP expand `/skill:<name>`, DSH a bare
/// whitespace-bounded `/<name>` token.
pub fn skill_command(backend: BackendKind, name: &str) -> String {
    match backend {
        BackendKind::Pi | BackendKind::Omp => format!("/skill:{name} "),
        BackendKind::Dsh => format!("/{name} "),
    }
}
