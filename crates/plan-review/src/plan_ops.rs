//! Pure operations on one chat's [`Plan`], shared by the Agent tools and the UI.
//!
//! Semantics follow the original ThinkRail TODO store: at most one `InProgress` item; leaving
//! `Done` clears that item's summary / verification / commit subject; `write_groups` reconciles only
//! Agent-origin steps and keeps user items and done items. `update` / `remove` do not check origin
//! (the Agent may edit user items, as in the original). The UI path uses [`remove_by_user`], which
//! refuses `Done` items because completed steps are the plan's audit trail.

use thinkrail_core::{Error, Origin, Plan, PlanGroup, PlanItem, PlanItemId, PlanStatus, Result};

const CARRIED_LOOSE_GROUP: &str = "Completed";
const MAX_TITLE: usize = 500;

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Collapses whitespace runs so a title is always one line.
fn flatten_title(raw: &str) -> Result<String> {
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(Error::invalid("标题不能为空"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(Error::invalid(format!("标题最长 {MAX_TITLE} 个字符")));
    }
    Ok(title)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn item(title: String, status: PlanStatus, origin: Origin, note: Option<String>) -> PlanItem {
    let now = now_ms();
    PlanItem {
        id: PlanItemId::generate(),
        title,
        status,
        origin,
        note: non_empty(note),
        summary: None,
        verification: None,
        commit_subject: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn items(plan: &Plan) -> impl Iterator<Item = &PlanItem> {
    plan.loose.iter().chain(plan.groups.iter().flat_map(|g| g.items.iter()))
}

fn items_mut(plan: &mut Plan) -> impl Iterator<Item = &mut PlanItem> {
    plan.loose.iter_mut().chain(plan.groups.iter_mut().flat_map(|g| g.items.iter_mut()))
}

pub fn find<'a>(plan: &'a Plan, id: &PlanItemId) -> Option<&'a PlanItem> {
    items(plan).find(|i| &i.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStatus { Pending, Active, Done }

/// Derived, never stored: all done → Done, any in progress → Active, otherwise Pending.
pub fn group_status(group: &PlanGroup) -> GroupStatus {
    if !group.items.is_empty() && group.items.iter().all(|i| i.status == PlanStatus::Done) {
        GroupStatus::Done
    } else if group.items.iter().any(|i| i.status == PlanStatus::InProgress) {
        GroupStatus::Active
    } else {
        GroupStatus::Pending
    }
}

/// Demotes every `InProgress` item except `keep` (or, without `keep`, except the first one).
fn keep_one_in_progress(plan: &mut Plan, keep: Option<&PlanItemId>) -> Vec<PlanItemId> {
    let mut demoted = Vec::new();
    let mut kept = false;
    let now = now_ms();
    for item in items_mut(plan) {
        if item.status != PlanStatus::InProgress {
            continue;
        }
        if keep.is_some_and(|k| k == &item.id) || (keep.is_none() && !kept) {
            kept = true;
            continue;
        }
        item.status = PlanStatus::Pending;
        item.updated_at = now;
        demoted.push(item.id.clone());
    }
    demoted
}

/// The store re-checks this before every save.
pub(crate) fn enforce_invariants(plan: &mut Plan) {
    keep_one_in_progress(plan, None);
}

#[derive(Debug, Clone, Default)]
pub struct NewItem {
    pub title: String,
    pub origin: Option<Origin>,
    /// Insert right after this item (in whichever lane it is).
    pub after: Option<PlanItemId>,
    /// Append to the group with this title, creating it if needed.
    pub group: Option<String>,
    pub note: Option<String>,
}

pub fn add(plan: &mut Plan, input: NewItem) -> Result<PlanItem> {
    let new = item(flatten_title(&input.title)?, PlanStatus::Pending, input.origin.unwrap_or(Origin::Agent), input.note);
    if let Some(after) = &input.after {
        let lane = std::iter::once(&mut plan.loose)
            .chain(plan.groups.iter_mut().map(|g| &mut g.items))
            .find(|lane| lane.iter().any(|i| &i.id == after))
            .ok_or_else(|| Error::not_found(format!("计划项 {after}")))?;
        let index = lane.iter().position(|i| &i.id == after).map_or(lane.len(), |i| i + 1);
        lane.insert(index, new.clone());
        return Ok(new);
    }
    match input.group.as_deref().map(flatten_title).transpose()? {
        Some(title) => {
            let index = match plan.groups.iter().position(|g| g.title == title) {
                Some(index) => index,
                None => {
                    plan.groups.push(PlanGroup { id: PlanItemId::generate(), title, items: Vec::new() });
                    plan.groups.len() - 1
                }
            };
            plan.groups[index].items.push(new.clone());
        }
        None => plan.loose.push(new.clone()),
    }
    Ok(new)
}

/// `Some(None)` clears a field; `None` leaves it untouched.
#[derive(Debug, Clone, Default)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub status: Option<PlanStatus>,
    pub note: Option<Option<String>>,
    pub summary: Option<Option<String>>,
    pub verification: Option<Option<String>>,
    pub commit_subject: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct Updated {
    pub item: PlanItem,
    /// Items auto-demoted to keep a single `InProgress`.
    pub paused: Vec<PlanItemId>,
}

pub fn update(plan: &mut Plan, id: &PlanItemId, patch: ItemPatch) -> Result<Updated> {
    let title = patch.title.as_deref().map(flatten_title).transpose()?;
    let target = items_mut(plan).find(|i| &i.id == id).ok_or_else(|| Error::not_found(format!("计划项 {id}")))?;
    if let Some(title) = title {
        target.title = title;
    }
    let mut becomes_active = false;
    if let Some(status) = patch.status {
        if target.status == PlanStatus::Done && status != PlanStatus::Done {
            target.summary = None;
            target.verification = None;
            target.commit_subject = None;
        }
        becomes_active = status == PlanStatus::InProgress;
        target.status = status;
    }
    if let Some(note) = patch.note {
        target.note = non_empty(note);
    }
    // Fresh done-time values in the same patch win over the clear above.
    if let Some(summary) = patch.summary {
        target.summary = non_empty(summary);
    }
    if let Some(verification) = patch.verification {
        target.verification = non_empty(verification);
    }
    if let Some(subject) = patch.commit_subject {
        target.commit_subject = non_empty(subject);
    }
    target.updated_at = now_ms();
    let paused = if becomes_active { keep_one_in_progress(plan, Some(id)) } else { Vec::new() };
    let item = find(plan, id).cloned().ok_or_else(|| Error::not_found(format!("计划项 {id}")))?;
    Ok(Updated { item, paused })
}

/// Agent `todo_remove` semantics: any item; a group emptied by this removal is dropped.
pub fn remove(plan: &mut Plan, id: &PlanItemId) -> Result<()> {
    let before = items(plan).count();
    plan.loose.retain(|i| &i.id != id);
    plan.groups.retain_mut(|group| {
        let had = group.items.len();
        group.items.retain(|i| &i.id != id);
        !(had > 0 && group.items.is_empty())
    });
    if items(plan).count() == before {
        return Err(Error::not_found(format!("计划项 {id}")));
    }
    Ok(())
}

/// UI removal: a `Done` item is the plan's audit trail and cannot be removed.
pub fn remove_by_user(plan: &mut Plan, id: &PlanItemId) -> Result<()> {
    if find(plan, id).is_some_and(|i| i.status == PlanStatus::Done) {
        return Err(Error::forbidden("已完成的计划项是审计记录，不能删除"));
    }
    remove(plan, id)
}

/// Turns a loose item into an empty group that keeps the item's id.
pub fn promote(plan: &mut Plan, id: &PlanItemId, title: Option<String>) -> Result<PlanGroup> {
    let Some(index) = plan.loose.iter().position(|i| &i.id == id) else {
        let grouped = plan.groups.iter().any(|g| g.items.iter().any(|i| &i.id == id));
        return Err(if grouped { Error::invalid("只能提升松散计划项") } else { Error::not_found(format!("计划项 {id}")) });
    };
    let title = flatten_title(title.as_deref().unwrap_or(&plan.loose[index].title))?;
    if plan.groups.iter().any(|g| g.title == title) {
        return Err(Error::conflict(format!("已有同名分组 {title}")));
    }
    let item = plan.loose.remove(index);
    let group = PlanGroup { id: item.id, title, items: Vec::new() };
    plan.groups.push(group.clone());
    Ok(group)
}

#[derive(Debug, Clone)]
pub struct WriteGroup {
    pub title: String,
    pub items: Vec<WriteItem>,
}

#[derive(Debug, Clone)]
pub struct WriteItem {
    pub title: String,
    pub status: Option<PlanStatus>,
    pub note: Option<String>,
}

/// Replaces the Agent's grouped plan. An existing Agent step matched by (group title, step title)
/// keeps its id and status (only its note is refreshed). User items and done Agent items are never
/// dropped: grouped user items move to the loose queue; unmatched done steps are carried along.
pub fn write_groups(plan: &mut Plan, groups: Vec<WriteGroup>) -> Result<()> {
    let current = std::mem::take(plan);
    let mut consumed = std::collections::HashSet::new();
    let mut fresh = Vec::with_capacity(groups.len());
    for group in groups {
        let title = flatten_title(&group.title)?;
        let mut items = Vec::with_capacity(group.items.len());
        for write in group.items {
            let step = flatten_title(&write.title)?;
            let hit = current
                .groups
                .iter()
                .filter(|g| g.title == title)
                .flat_map(|g| g.items.iter())
                .find(|i| i.origin == Origin::Agent && i.title == step && !consumed.contains(&i.id));
            items.push(match hit {
                Some(existing) => {
                    consumed.insert(existing.id.clone());
                    let mut merged = existing.clone();
                    if let Some(note) = write.note {
                        merged.note = non_empty(Some(note));
                        merged.updated_at = now_ms();
                    }
                    merged
                }
                None => item(step, write.status.unwrap_or(PlanStatus::Pending), Origin::Agent, write.note),
            });
        }
        fresh.push(PlanGroup { id: PlanItemId::generate(), title, items });
    }
    let mut loose: Vec<PlanItem> =
        current.loose.iter().filter(|i| i.origin == Origin::User || i.status != PlanStatus::Done).cloned().collect();
    let mut carried = Vec::new();
    for old in &current.groups {
        if old.items.is_empty() {
            if !fresh.iter().any(|g| g.title == old.title) {
                carried.push(old.clone());
            }
            continue;
        }
        let mut done = Vec::new();
        for i in &old.items {
            if consumed.contains(&i.id) {
                continue;
            }
            if i.origin == Origin::User {
                loose.push(i.clone());
            } else if i.status == PlanStatus::Done {
                match fresh.iter_mut().find(|g| g.title == old.title) {
                    Some(group) => group.items.push(i.clone()),
                    None => done.push(i.clone()),
                }
            }
        }
        if !done.is_empty() {
            carried.push(PlanGroup { id: PlanItemId::generate(), title: old.title.clone(), items: done });
        }
    }
    let loose_done: Vec<PlanItem> =
        current.loose.iter().filter(|i| i.origin == Origin::Agent && i.status == PlanStatus::Done).cloned().collect();
    if !loose_done.is_empty() {
        match fresh.iter_mut().find(|g| g.title == CARRIED_LOOSE_GROUP) {
            Some(group) => group.items.extend(loose_done),
            None => carried.push(PlanGroup { id: PlanItemId::generate(), title: CARRIED_LOOSE_GROUP.into(), items: loose_done }),
        }
    }
    fresh.extend(carried);
    *plan = Plan { groups: fresh, loose, summary: current.summary };
    keep_one_in_progress(plan, None);
    Ok(())
}

pub fn set_summary(plan: &mut Plan, summary: &str) {
    plan.summary = non_empty(Some(summary.to_owned()));
}

#[cfg(test)]
mod tests;
