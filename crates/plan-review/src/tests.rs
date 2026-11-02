use std::{fs, path::PathBuf, sync::Arc};

use tempfile::TempDir;
use thinkrail_core::{
    ErrorKind, NewReviewComment, Origin, PlanReviewService, PlanScope, PlanStatus, Result,
    ReviewState, WorkspaceId, WorkspaceRoots,
};

use crate::{
    JsonPlanReviewStore,
    plan_ops::{self, ItemPatch, NewItem},
};

struct Fixed(PathBuf);
impl WorkspaceRoots for Fixed {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

fn scope(workspace: &str, chat: &str) -> PlanScope {
    PlanScope {
        workspace: workspace.into(),
        chat: chat.into(),
    }
}

fn open(data: &TempDir, work: &TempDir) -> JsonPlanReviewStore {
    JsonPlanReviewStore::open(data.path(), Arc::new(Fixed(work.path().to_path_buf()))).unwrap()
}

#[test]
fn plans_persist_per_scope_and_across_instances() {
    let (data, work) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let first = open(&data, &work);
    let second = open(&data, &work);
    let a = scope("w:1", "c");
    let b = scope("w", "1:c");
    first
        .update_plan(&a, &mut |plan| {
            plan_ops::add(
                plan,
                NewItem {
                    title: "Mine".into(),
                    origin: Some(Origin::User),
                    ..NewItem::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
    // A second instance re-reads under the lock, so neither write is lost.
    second
        .update_plan(&a, &mut |plan| {
            let id = plan_ops::add(
                plan,
                NewItem {
                    title: "Agent".into(),
                    ..NewItem::default()
                },
            )?
            .id;
            plan_ops::update(
                plan,
                &id,
                ItemPatch {
                    status: Some(PlanStatus::InProgress),
                    ..ItemPatch::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(first.plan(&a).unwrap().loose.len(), 2);
    assert!(
        first.plan(&b).unwrap().loose.is_empty(),
        "scopes with colons never collide"
    );
    let failed = first.update_plan(&a, &mut |plan| {
        plan_ops::add(
            plan,
            NewItem {
                title: "Lost".into(),
                ..NewItem::default()
            },
        )?;
        Err(thinkrail_core::Error::invalid("abort"))
    });
    assert!(failed.is_err());
    assert_eq!(
        first.plan(&a).unwrap().loose.len(),
        2,
        "failed change is not persisted"
    );
}

#[test]
fn anchors_relocate_go_stale_and_block_sending() {
    let (data, work) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let store = open(&data, &work);
    let s = scope("w", "c");
    let file = work.path().join("a.rs");
    fs::write(&file, "fn one() {}\nlet target = 1;\n").unwrap();
    let bad = NewReviewComment {
        path: "a.rs".into(),
        line: 1,
        text_quote: Some("target".into()),
        body: "x".into(),
    };
    assert_eq!(
        store.add_review(&s, bad).unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    let escape = NewReviewComment {
        path: "../a.rs".into(),
        line: 1,
        text_quote: None,
        body: "x".into(),
    };
    assert_eq!(
        store.add_review(&s, escape).unwrap_err().kind(),
        Some(ErrorKind::Forbidden)
    );
    let good = NewReviewComment {
        path: "a.rs".into(),
        line: 2,
        text_quote: Some("target".into()),
        body: "改 <这里>".into(),
    };
    let comment = store.add_review(&s, good).unwrap();
    assert_eq!(comment.state, ReviewState::Draft);
    fs::write(&file, "// header\nfn one() {}\nlet target = 1;\n").unwrap();
    let listed = store.reviews(&s).unwrap();
    assert_eq!((listed[0].line, listed[0].stale), (3, false));
    let package = store.send_package(&s).unwrap();
    assert_eq!(package.comment_count, 1);
    assert!(package.prompt.contains("改 &lt;这里&gt;"));
    assert_eq!(store.reviews(&s).unwrap()[0].state, ReviewState::Sent);
    fs::write(&file, "let target = 1;\nlet target = 2;\n").unwrap();
    assert!(store.reviews(&s).unwrap()[0].stale);
    assert_eq!(
        store.send_package(&s).unwrap_err().kind(),
        Some(ErrorKind::Conflict)
    );
    store.resolve_review(&s, &comment.id).unwrap();
    assert_eq!(store.send_package(&s).unwrap().comment_count, 0);
    store.remove_review(&s, &comment.id).unwrap();
    assert!(store.reviews(&s).unwrap().is_empty());
}

#[test]
fn corrupt_storage_is_rejected_and_never_overwritten() {
    let (data, work) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let store = open(&data, &work);
    for damaged in ["{broken", r#"{"schemaVersion":7,"scopes":{}}"#] {
        fs::write(data.path().join("review-plan.json"), damaged).unwrap();
        assert!(
            JsonPlanReviewStore::open(data.path(), Arc::new(Fixed(work.path().to_path_buf())))
                .is_err()
        );
        let result = store.update_plan(&scope("w", "c"), &mut |_| Ok(()));
        assert_eq!(result.unwrap_err().kind(), Some(ErrorKind::Corrupt));
        assert_eq!(
            fs::read_to_string(data.path().join("review-plan.json")).unwrap(),
            damaged
        );
    }
}
