use thinkrail_core::{ErrorKind, Origin, Plan, PlanStatus};

use super::*;

fn add_titled(plan: &mut Plan, title: &str, group: Option<&str>, origin: Origin) -> PlanItemId {
    add(plan, NewItem { title: title.into(), group: group.map(Into::into), origin: Some(origin), ..NewItem::default() })
        .unwrap()
        .id
}

fn status(plan: &Plan, id: &PlanItemId) -> PlanStatus {
    find(plan, id).unwrap().status
}

fn set(plan: &mut Plan, id: &PlanItemId, status: PlanStatus) -> Updated {
    update(plan, id, ItemPatch { status: Some(status), ..ItemPatch::default() }).unwrap()
}

#[test]
fn only_one_item_is_in_progress_and_paused_items_are_reported() {
    let mut plan = Plan::default();
    let a = add_titled(&mut plan, "A", Some("G"), Origin::Agent);
    let b = add_titled(&mut plan, "B", None, Origin::User);
    set(&mut plan, &a, PlanStatus::InProgress);
    let updated = set(&mut plan, &b, PlanStatus::InProgress);
    assert_eq!(updated.paused, vec![a.clone()]);
    assert_eq!((status(&plan, &a), status(&plan, &b)), (PlanStatus::Pending, PlanStatus::InProgress));
    assert_eq!(group_status(&plan.groups[0]), GroupStatus::Pending);
}

#[test]
fn leaving_done_clears_completion_fields_but_keeps_plan_summary() {
    let mut plan = Plan::default();
    let a = add_titled(&mut plan, "A", None, Origin::Agent);
    update(&mut plan, &a, ItemPatch {
        status: Some(PlanStatus::Done),
        summary: Some(Some("did it".into())),
        verification: Some(Some("tests".into())),
        commit_subject: Some(Some("feat: a".into())),
        ..ItemPatch::default()
    })
    .unwrap();
    set_summary(&mut plan, "whole plan");
    let reopened = set(&mut plan, &a, PlanStatus::Pending).item;
    assert_eq!((reopened.summary, reopened.verification, reopened.commit_subject), (None, None, None));
    assert_eq!(plan.summary.as_deref(), Some("whole plan"));
}

#[test]
fn write_groups_keeps_user_items_and_done_steps() {
    let mut plan = Plan::default();
    let kept = add_titled(&mut plan, "Step", Some("Build"), Origin::Agent);
    set(&mut plan, &kept, PlanStatus::InProgress);
    let dropped = add_titled(&mut plan, "Old step", Some("Build"), Origin::Agent);
    let done = add_titled(&mut plan, "Shipped", Some("Old"), Origin::Agent);
    set(&mut plan, &done, PlanStatus::Done);
    let user_grouped = add_titled(&mut plan, "Mine", Some("Build"), Origin::User);
    let user_loose = add_titled(&mut plan, "Also mine", None, Origin::User);
    write_groups(&mut plan, vec![WriteGroup {
        title: "Build".into(),
        items: vec![
            WriteItem { title: "Step".into(), status: Some(PlanStatus::Pending), note: Some("refreshed".into()) },
            WriteItem { title: "New".into(), status: None, note: None },
        ],
    }])
    .unwrap();
    let step = find(&plan, &kept).unwrap();
    assert_eq!((step.status, step.note.as_deref()), (PlanStatus::InProgress, Some("refreshed")));
    assert!(find(&plan, &dropped).is_none());
    assert_eq!(status(&plan, &done), PlanStatus::Done);
    assert!(plan.loose.iter().any(|i| i.id == user_grouped));
    assert!(plan.loose.iter().any(|i| i.id == user_loose));
    assert!(items(&plan).filter(|i| i.status == PlanStatus::InProgress).count() <= 1);
}

#[test]
fn agent_may_remove_anything_but_user_cannot_remove_done() {
    let mut plan = Plan::default();
    let user = add_titled(&mut plan, "Mine", None, Origin::User);
    let done = add_titled(&mut plan, "Done", Some("G"), Origin::Agent);
    set(&mut plan, &done, PlanStatus::Done);
    assert_eq!(remove_by_user(&mut plan, &done).unwrap_err().kind(), Some(ErrorKind::Forbidden));
    remove(&mut plan, &user).unwrap();
    remove(&mut plan, &done).unwrap();
    assert!(plan.groups.is_empty(), "group emptied by removal is dropped");
    assert_eq!(remove(&mut plan, &done).unwrap_err().kind(), Some(ErrorKind::NotFound));
}

#[test]
fn promote_and_insert_after() {
    let mut plan = Plan::default();
    let loose = add_titled(&mut plan, "Idea", None, Origin::Agent);
    let group = promote(&mut plan, &loose, Some("Feature".into())).unwrap();
    assert_eq!((group.id, plan.loose.len()), (loose, 0));
    let first = add_titled(&mut plan, "One", Some("Feature"), Origin::Agent);
    let third = add_titled(&mut plan, "Three", Some("Feature"), Origin::Agent);
    add(&mut plan, NewItem { title: "Two".into(), after: Some(first.clone()), ..NewItem::default() }).unwrap();
    let titles: Vec<_> = plan.groups[0].items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["One", "Two", "Three"]);
    assert_eq!(promote(&mut plan, &third, None).unwrap_err().kind(), Some(ErrorKind::Invalid));
    assert_eq!(add(&mut plan, NewItem { title: "  ".into(), ..NewItem::default() }).unwrap_err().kind(), Some(ErrorKind::Invalid));
}
