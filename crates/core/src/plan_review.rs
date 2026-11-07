use serde::{Deserialize, Serialize};

use crate::{Baseline, ChatId, ItemChange, PlanItemId, Result, ReviewId, StepReviews, WorkspaceId};

/// Plans and reviews are scoped to one chat inside one workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanScope {
    pub workspace: WorkspaceId,
    pub chat: ChatId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Added in the UI; `todo_write` never overwrites it.
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub id: PlanItemId,
    pub title: String,
    pub status: PlanStatus,
    pub origin: Origin,
    pub note: Option<String>,
    /// Set only while `Done`; cleared when the item leaves `Done`.
    pub summary: Option<String>,
    pub verification: Option<String>,
    pub commit_subject: Option<String>,
    /// The step's change set (agent-settings 08 §5.8); kept when the step leaves `Done`, so a fix
    /// round appends to it.
    #[serde(default)]
    pub changes: Vec<ItemChange>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A named group of steps. Its status is derived from its items, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanGroup {
    pub id: PlanItemId,
    pub title: String,
    pub items: Vec<PlanItem>,
}

/// One chat's plan. Invariant: at most one item across `groups` and `loose` is `InProgress`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub groups: Vec<PlanGroup>,
    /// Ungrouped queue.
    pub loose: Vec<PlanItem>,
    /// Plan-level summary in PR-body form.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    Draft,
    /// Included in a package sent to the chat; only these can be resolved by an Agent.
    Sent,
    Resolved,
}

/// A review comment anchored to a line of a workspace file.
///
/// The anchor is re-located on every read: first by the original line's hash, then by a unique
/// `text_quote` match. Ambiguous or missing anchors are `stale` and block sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: ReviewId,
    pub path: String,
    /// 1-based.
    pub line: u32,
    pub text_quote: String,
    pub line_hash: String,
    pub file_hash: String,
    pub body: String,
    pub state: ReviewState,
    pub stale: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReviewComment {
    pub path: String,
    pub line: u32,
    /// Must occur on `line`; `None` uses the whole line.
    pub text_quote: Option<String>,
    pub body: String,
}

/// The prompt sent to the chat for a review package, plus how many comments it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPackage {
    pub prompt: String,
    pub comment_count: usize,
}

/// Durable plans and review comments. Writes re-read under an OS lock, so several app instances
/// never lose each other's updates; damaged storage is reported, never overwritten.
pub trait PlanReviewService: Send + Sync {
    fn plan(&self, scope: &PlanScope) -> Result<Plan>;
    /// Atomic read-modify-write of one chat's plan under the store lock. The closure enforces
    /// tool/UI semantics; the store re-validates the single-`InProgress` invariant before saving.
    fn update_plan(
        &self,
        scope: &PlanScope,
        change: &mut dyn FnMut(&mut Plan) -> Result<()>,
    ) -> Result<Plan>;

    fn reviews(&self, scope: &PlanScope) -> Result<Vec<ReviewComment>>;
    fn add_review(&self, scope: &PlanScope, comment: NewReviewComment) -> Result<ReviewComment>;
    fn update_review(
        &self,
        scope: &PlanScope,
        id: &ReviewId,
        body: String,
    ) -> Result<ReviewComment>;
    fn resolve_review(&self, scope: &PlanScope, id: &ReviewId) -> Result<ReviewComment>;
    fn remove_review(&self, scope: &PlanScope, id: &ReviewId) -> Result<()>;
    /// Fails with `Conflict` while any unresolved comment is stale; marks sent comments `Sent`.
    fn send_package(&self, scope: &PlanScope) -> Result<ReviewPackage>;

    /// The chat's plan-step review state (08 §5.6, §5.8).
    fn step_reviews(&self, scope: &PlanScope) -> Result<StepReviews>;
    /// Atomic read-modify-write of the plan and its review state together, under the store lock.
    fn update_reviews(
        &self,
        scope: &PlanScope,
        change: &mut dyn FnMut(&mut Plan, &mut StepReviews) -> Result<()>,
    ) -> Result<(Plan, StepReviews)>;
    /// Drops every `pending` mark: no review survives a restart (the original's boot reconcile).
    fn clear_pending_reviews(&self) -> Result<()>;
    /// Opens `item`'s work window (08 §5.8) if it is still `InProgress` and has none. When another
    /// chat of the workspace has a window open, both sides are marked `shared` in the same write.
    /// Returns the stored baseline, `None` when nothing was opened.
    fn open_window(
        &self,
        scope: &PlanScope,
        item: &PlanItemId,
        baseline: Baseline,
    ) -> Result<Option<Baseline>>;
    /// Whether another chat of the workspace has a work window open.
    fn windows_elsewhere(&self, scope: &PlanScope) -> Result<bool>;
}
