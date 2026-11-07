use thinkrail_core::{
    FindingState, ItemChange, Origin, PlanItem, PlanStatus, ReviewFinding, ReviewOutcome,
    ReviewRecord, Verdict,
};

use crate::review::{Subject, fix_package, result_text, review_package};

fn step(changes: Vec<ItemChange>) -> PlanItem {
    PlanItem {
        id: "p1".into(),
        title: "Add {chat} guard".into(),
        status: PlanStatus::Done,
        origin: Origin::Agent,
        note: Some("keep {note} literal".into()),
        summary: None,
        verification: None,
        commit_subject: None,
        changes,
        created_at: 0,
        updated_at: 0,
    }
}

fn commit(sha: &str) -> ItemChange {
    ItemChange::Commit {
        sha: sha.into(),
        label: "x".into(),
    }
}

fn record(reviewed: &[&str]) -> ReviewRecord {
    ReviewRecord {
        verdict: Verdict::RequestChanges,
        summary: None,
        findings: Vec::new(),
        reviewed_shas: reviewed.iter().map(|s| s.to_string()).collect(),
        model: None,
        at: 0,
    }
}

const A: &str = "aaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbb";

#[test]
fn package_substitutes_once_and_reports_missing_verification() {
    let item = step(vec![commit(A)]);
    let text = review_package(Subject::Step(&item), "chat-1", None).unwrap();
    assert!(text.starts_with("Plan step p1 (\"Add {chat} guard\") of chat chat-1 is done"));
    assert!(text.contains("Step note: keep {note} literal"));
    assert!(text.contains("Worker reported NO verification"));
    assert!(text.contains("Change set: commit aaaaaaaaaaaa\n"));
    assert!(text.ends_with("described in your instructions."));
    assert!(!text.contains("RE-REVIEW"));
}

#[test]
fn rereview_names_only_new_commits_and_needs_a_partial_overlap() {
    let item = step(vec![commit(A), commit(B)]);
    let subject = Subject::Step(&item);
    let partial = review_package(subject, "c", Some(&record(&[A]))).unwrap();
    assert!(partial.contains("RE-REVIEW: only bbbbbbbbbbbb is new"));
    // Nothing reviewed before (or history rewritten): a full review, not a delta.
    let none_seen = review_package(subject, "c", Some(&record(&["zzz"]))).unwrap();
    assert!(!none_seen.contains("RE-REVIEW"));
    let all_seen = review_package(subject, "c", Some(&record(&[A, B]))).unwrap();
    assert!(!all_seen.contains("RE-REVIEW"));
}

#[test]
fn change_sets_mix_commits_and_paths_and_nothing_is_unreviewable() {
    let path = |p: &str| ItemChange::Path { path: p.into() };
    let both = step(vec![commit(A), commit(B), path("src/a.rs")]);
    assert_eq!(
        Subject::Step(&both).change_set().unwrap(),
        "commits aaaaaaaaaaaa, bbbbbbbbbbbb; uncommitted paths: src/a.rs"
    );
    let paths = step(vec![path("a"), path("b")]);
    assert_eq!(
        Subject::Step(&paths).change_set().unwrap(),
        "changed paths: a, b"
    );
    let empty = step(Vec::new());
    assert!(review_package(Subject::Step(&empty), "c", None).is_none());
    assert!(fix_package(Subject::Step(&empty), &[]).is_none());
    let orphan = review_package(
        Subject::Commit {
            sha: A,
            title: "Fix",
        },
        "c",
        None,
    )
    .unwrap();
    assert!(orphan.starts_with("Commit aaaaaaaaaaaa (\"Fix\") of chat c belongs to no plan step"));
}

#[test]
fn result_text_states_the_next_action_and_lists_findings_with_locations() {
    let finding = |id: &str, path: Option<&str>, start, end| ReviewFinding {
        id: id.into(),
        path: path.map(Into::into),
        start_line: start,
        end_line: end,
        body: "BUG: x".into(),
        state: FindingState::Open,
    };
    let findings = [
        finding("rf_1", Some("a.rs"), Some(3), Some(5)),
        finding("rf_2", Some("a.rs"), Some(7), Some(7)),
        finding("rf_3", None, None, None),
    ];
    let fix = result_text("T", &ReviewOutcome::ChangesFix, Some("why"), &findings);
    assert_eq!(
        fix,
        "Review verdict: REQUEST_CHANGES — step \"T\". Fix each finding below NOW without asking \
         the user: set the step back to in_progress (todo_update), apply the fixes, mark it done, \
         then call request_review again.\n\nwhy\n\n- [rf_1] (a.rs:3-5) BUG: x\n- [rf_2] (a.rs:7) \
         BUG: x\n- [rf_3] BUG: x"
    );
    let stop = result_text("T", &ReviewOutcome::ChangesStop, None, &findings[..1]);
    assert!(stop.contains("do NOT fix now"));
    let blocked = result_text("T", &ReviewOutcome::ApproveBlocked { open: 2 }, None, &[]);
    assert!(blocked.contains("2 earlier finding(s)") && blocked.contains("resolve_comment"));
    let failed = result_text(
        "T",
        &ReviewOutcome::Failed {
            message: "timeout".into(),
        },
        Some("ignored"),
        &findings,
    );
    assert_eq!(failed, "Review of step \"T\" failed: timeout");
}

#[test]
fn fix_package_quotes_the_findings_and_reopens_the_step() {
    let item = step(vec![commit(A)]);
    let finding = ReviewFinding {
        id: "rf_1".into(),
        path: None,
        start_line: None,
        end_line: None,
        body: "BUG: y".into(),
        state: FindingState::Open,
    };
    let text = fix_package(Subject::Step(&item), &[finding]).unwrap();
    assert!(text.contains("User feedback:\n\"\"\"\n- [rf_1] BUG: y\n\"\"\""));
    assert!(text.ends_with("Do not create a new item for it."));
    assert!(text.contains("flip p1 back to in_progress"));
}
