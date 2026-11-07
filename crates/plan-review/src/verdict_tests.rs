use thinkrail_core::{
    ItemChange, Origin, PlanItem, PlanItemId, PlanStatus, ReviewOutcome, StepReviews, Verdict,
};

use crate::verdict::{
    NewFinding, Parsed, apply, fail, needs_auto_rereview, parse, resolve_finding,
};

const CHANGES: &str = r#"{"verdict":"request_changes","summary":"s","findings":[{"id":"f1","path":"a\\b.rs","startLine":2,"endLine":4,"body":"BUG: x"}]}"#;

#[test]
fn parse_takes_the_first_json_block_or_the_whole_reply() {
    let fenced = format!("Looked at it.\n```JSON\n{CHANGES}\n```\ntrailing ```json {{}} ```");
    let parsed = parse(&fenced).unwrap();
    assert_eq!(parsed.verdict, Verdict::RequestChanges);
    assert_eq!(parsed.summary.as_deref(), Some("s"));
    assert_eq!(
        parsed.findings,
        [NewFinding {
            path: Some("a/b.rs".into()),
            start_line: Some(2),
            end_line: Some(4),
            body: "BUG: x".into(),
        }]
    );
    assert_eq!(parse(&format!("  {CHANGES}\n")), Some(parsed));
    // Unknown fields are ignored; an approval may have no findings.
    let approve = parse(r#"{"verdict":"approve","extra":1,"findings":[]}"#).unwrap();
    assert_eq!(approve.verdict, Verdict::Approve);
    assert!(approve.summary.is_none());
}

#[test]
fn parse_rejects_contract_breaks() {
    let finding = |extra: &str| {
        format!(r#"{{"verdict":"approve","findings":[{{"id":"f","body":"b"{extra}}}]}}"#)
    };
    assert!(parse(&finding("")).is_some());
    for bad in [
        String::new(),
        "not json".into(),
        r#"{"verdict":"maybe","findings":[]}"#.into(),
        r#"{"verdict":"approve"}"#.into(),
        r#"{"verdict":"request_changes","findings":[]}"#.into(),
        r#"{"verdict":"approve","summary":3,"findings":[]}"#.into(),
        r#"{"verdict":"approve","findings":[{"id":"","body":"b"}]}"#.into(),
        r#"{"verdict":"approve","findings":[{"id":"f","body":""}]}"#.into(),
        finding(r#","kind":"nope""#),
        finding(r#","startLine":3"#),
        finding(r#","path":"a","endLine":3"#),
        finding(r#","path":"a","startLine":0"#),
        finding(r#","path":"a","startLine":1.5"#),
        finding(r#","path":"a","startLine":5,"endLine":3"#),
        format!(
            r#"{{"verdict":"approve","findings":[],"readingOrder":[{}]}}"#,
            vec![r#"{"path":"a","why":"w"}"#; 13].join(",")
        ),
        r#"{"verdict":"approve","findings":[],"readingOrder":[{"path":"a"}]}"#.into(),
    ] {
        assert!(parse(&bad).is_none(), "{bad}");
    }
}

fn changes(n: usize) -> Parsed {
    Parsed {
        verdict: Verdict::RequestChanges,
        summary: None,
        findings: (0..n)
            .map(|i| NewFinding {
                path: None,
                start_line: None,
                end_line: None,
                body: format!("BUG: {i}"),
            })
            .collect(),
    }
}

fn approve() -> Parsed {
    Parsed {
        verdict: Verdict::Approve,
        summary: Some("ok".into()),
        findings: Vec::new(),
    }
}

fn reviewing(steps: &mut StepReviews, item: &PlanItemId, shas: &[&str]) {
    steps
        .pending
        .insert(item.clone(), shas.iter().map(|s| s.to_string()).collect());
}

#[test]
fn request_changes_fixes_once_then_hands_over_to_the_user() {
    let item = PlanItemId::from("p");
    let mut steps = StepReviews::default();
    reviewing(&mut steps, &item, &["a"]);
    let first = apply(&mut steps, &item, changes(2), true, None, 1);
    assert_eq!(first.outcome, ReviewOutcome::ChangesFix);
    assert_eq!(first.findings.len(), 2);
    assert!(first.findings.iter().all(|f| f.id.starts_with("rf_")));
    assert_eq!(steps.auto_cycles[&item], 1);
    assert!(steps.pending.is_empty());
    assert_eq!(steps.verdicts[&item].reviewed_shas, ["a"]);

    // The re-review still asks for changes: the round is spent. Unresolved findings carry over.
    reviewing(&mut steps, &item, &["a", "b"]);
    let second = apply(&mut steps, &item, changes(1), true, None, 2);
    assert_eq!(second.outcome, ReviewOutcome::ChangesStop);
    assert_eq!(second.findings.len(), 1);
    assert_eq!(steps.auto_cycles[&item], 2);
    assert_eq!(steps.verdicts[&item].findings.len(), 3);

    // Auto-fix off never spends a round on fixing.
    let other = PlanItemId::from("q");
    let off = apply(&mut steps, &other, changes(1), false, None, 3);
    assert_eq!(off.outcome, ReviewOutcome::ChangesStop);
    assert_eq!(steps.auto_cycles[&other], 2);
}

#[test]
fn approve_is_blocked_by_open_findings_and_otherwise_settles_the_step() {
    let item = PlanItemId::from("p");
    let mut steps = StepReviews::default();
    apply(&mut steps, &item, changes(2), true, None, 1);
    let ids: Vec<String> = steps.verdicts[&item]
        .findings
        .iter()
        .map(|f| f.id.clone())
        .collect();

    reviewing(&mut steps, &item, &["b"]);
    let blocked = apply(&mut steps, &item, approve(), true, None, 2);
    assert_eq!(blocked.outcome, ReviewOutcome::ApproveBlocked { open: 2 });
    // The record and the fix round are unchanged; the review is no longer pending.
    assert_eq!(steps.verdicts[&item].verdict, Verdict::RequestChanges);
    assert_eq!(steps.auto_cycles[&item], 1);
    assert!(steps.pending.is_empty());

    assert!(resolve_finding(&mut steps, &ids[0]));
    assert!(!resolve_finding(&mut steps, &ids[0]), "already resolved");
    assert!(steps.verdicts.contains_key(&item));
    reviewing(&mut steps, &item, &["a", "b"]);
    assert_eq!(
        apply(&mut steps, &item, approve(), true, None, 3).outcome,
        ReviewOutcome::ApproveBlocked { open: 1 }
    );
    assert!(resolve_finding(&mut steps, &ids[1]));
    // The last open finding resolved: the step goes back to unreviewed, the round stays spent.
    assert!(!steps.verdicts.contains_key(&item));
    assert_eq!(steps.auto_cycles[&item], 1);

    reviewing(&mut steps, &item, &["a", "b"]);
    let approved = apply(&mut steps, &item, approve(), true, None, 4);
    assert_eq!(approved.outcome, ReviewOutcome::Approved);
    assert!(!steps.auto_cycles.contains_key(&item));
    assert_eq!(steps.verdicts[&item].reviewed_shas, ["a", "b"]);
    assert!(!resolve_finding(&mut steps, "rf_unknown"));
}

#[test]
fn a_failed_review_only_ends_the_pending_mark() {
    let item = PlanItemId::from("p");
    let mut steps = StepReviews::default();
    apply(&mut steps, &item, changes(1), true, None, 1);
    let before = steps.clone();
    reviewing(&mut steps, &item, &["b"]);
    fail(&mut steps, &item);
    assert_eq!(steps, before);
}

#[test]
fn auto_rereview_needs_the_fix_round_a_done_step_and_unseen_commits() {
    let commit = |sha: &str| ItemChange::Commit {
        sha: sha.into(),
        label: "x".into(),
    };
    let mut item = PlanItem {
        id: "p".into(),
        title: "t".into(),
        status: PlanStatus::Done,
        origin: Origin::Agent,
        note: None,
        summary: None,
        verification: None,
        commit_subject: None,
        changes: vec![commit("a")],
        created_at: 0,
        updated_at: 0,
    };
    let mut steps = StepReviews::default();
    reviewing(&mut steps, &item.id, &["a"]);
    apply(&mut steps, &item.id, changes(1), true, None, 1);
    assert!(!needs_auto_rereview(&steps, &item), "no new commit yet");
    item.changes.push(commit("b"));
    assert!(needs_auto_rereview(&steps, &item));
    item.status = PlanStatus::InProgress;
    assert!(!needs_auto_rereview(&steps, &item));
    item.status = PlanStatus::Done;
    reviewing(&mut steps, &item.id, &["a", "b"]);
    assert!(!needs_auto_rereview(&steps, &item), "already reviewing");
    fail(&mut steps, &item.id);
    // Only paths: the record was dropped, so any change set qualifies.
    item.changes = vec![ItemChange::Path { path: "a".into() }];
    steps.verdicts.clear();
    assert!(needs_auto_rereview(&steps, &item));
    steps.auto_cycles.insert(item.id.clone(), 2);
    assert!(!needs_auto_rereview(&steps, &item), "the round is over");
}
