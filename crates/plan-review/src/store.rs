use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use thinkrail_core::{
    Baseline, Error, IoContext, NewReviewComment, Plan, PlanItemId, PlanReviewService, PlanScope,
    PlanStatus, Result, ReviewComment, ReviewId, ReviewPackage, ReviewState, StepReviews,
    WorkspaceRoots,
};
use tracing::debug;

use crate::{anchor, plan_ops};

const FILE_NAME: &str = "review-plan.json";
const LOCK_NAME: &str = "review-plan.json.lock";
const SCHEMA_VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Scopes are stored as `workspace` → `chat` maps so ids containing any character cannot collide.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    schema_version: u32,
    scopes: BTreeMap<String, BTreeMap<String, ScopeData>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeData {
    plan: Plan,
    reviews: Vec<ReviewComment>,
    /// Plan-step reviews and work windows (agent-settings 08 §5.6, §5.8). Absent in files written
    /// before they existed; an older binary reports a file with them as damaged (downgrade only).
    #[serde(default, skip_serializing_if = "is_empty")]
    steps: StepReviews,
}

fn is_empty(steps: &StepReviews) -> bool {
    *steps == StepReviews::default()
}

impl Document {
    fn get(&self, scope: &PlanScope) -> ScopeData {
        self.scopes
            .get(scope.workspace.as_str())
            .and_then(|chats| chats.get(scope.chat.as_str()))
            .cloned()
            .unwrap_or_default()
    }

    fn put(&mut self, scope: &PlanScope, data: ScopeData) {
        self.scopes
            .entry(scope.workspace.to_string())
            .or_default()
            .insert(scope.chat.to_string(), data);
    }
}

/// [`PlanReviewService`] backed by `<data>/review-plan.json`, written under an fs2 lock with a
/// re-read so several app instances never lose each other's updates.
pub struct JsonPlanReviewStore {
    dir: PathBuf,
    roots: Arc<dyn WorkspaceRoots>,
}

impl std::fmt::Debug for JsonPlanReviewStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonPlanReviewStore")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl JsonPlanReviewStore {
    /// Damaged storage fails with `Corrupt` and is left untouched.
    pub fn open(data_dir: &Path, roots: Arc<dyn WorkspaceRoots>) -> Result<Self> {
        fs::create_dir_all(data_dir).context("创建数据目录")?;
        let store = Self {
            dir: data_dir.to_path_buf(),
            roots,
        };
        store.read()?;
        Ok(store)
    }

    fn read(&self) -> Result<Document> {
        let path = self.dir.join(FILE_NAME);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Document {
                    schema_version: SCHEMA_VERSION,
                    ..Document::default()
                });
            }
            Err(error) => {
                return Err(Error::Io {
                    context: "读取计划与评审数据".into(),
                    source: error,
                });
            }
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("读取计划与评审数据")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::corrupt("计划与评审数据超过 64 MiB"));
        }
        let document: Document =
            serde_json::from_slice(&bytes).map_err(|_| Error::corrupt("计划与评审数据无法解析"))?;
        if document.schema_version != SCHEMA_VERSION {
            return Err(Error::corrupt(format!(
                "计划与评审数据版本 {} 不受支持",
                document.schema_version
            )));
        }
        Ok(document)
    }

    /// The cross-process lock every write holds; released when dropped.
    fn lock(&self) -> Result<File> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(LOCK_NAME))
            .context("打开计划与评审锁")?;
        fs2::FileExt::lock_exclusive(&lock).context("锁定计划与评审数据")?;
        Ok(lock)
    }

    /// Lock → re-read → change one scope → validate → atomic replace. Nothing is written if the
    /// change fails.
    fn mutate<T>(
        &self,
        scope: &PlanScope,
        change: impl FnOnce(&mut ScopeData) -> Result<T>,
    ) -> Result<T> {
        let lock = self.lock()?;
        let mut document = self.read()?;
        let mut data = document.get(scope);
        let value = change(&mut data)?;
        plan_ops::enforce_invariants(&mut data.plan);
        document.put(scope, data);
        self.write(&document)?;
        drop(lock);
        Ok(value)
    }

    fn write(&self, document: &Document) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|_| Error::invalid("计划与评审数据无法序列化"))?;
        let target = self.dir.join(FILE_NAME);
        let temporary = self
            .dir
            .join(format!("{FILE_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &target)
        })()
        .context("保存计划与评审数据");
        if result.is_err()
            && let Err(error) = fs::remove_file(&temporary)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(code = "IO", io_kind = ?error.kind(), "review.store.cleanup_failed");
        }
        result
    }

    fn find(reviews: &mut [ReviewComment], id: &ReviewId) -> Result<usize> {
        reviews
            .iter()
            .position(|r| &r.id == id)
            .ok_or_else(|| Error::not_found("评审评论"))
    }
}

impl PlanReviewService for JsonPlanReviewStore {
    fn plan(&self, scope: &PlanScope) -> Result<Plan> {
        Ok(self.read()?.get(scope).plan)
    }

    fn update_plan(
        &self,
        scope: &PlanScope,
        change: &mut dyn FnMut(&mut Plan) -> Result<()>,
    ) -> Result<Plan> {
        self.mutate(scope, |data| {
            change(&mut data.plan)?;
            Ok(())
        })?;
        self.plan(scope)
    }

    fn reviews(&self, scope: &PlanScope) -> Result<Vec<ReviewComment>> {
        let root = self.roots.workspace_root(&scope.workspace)?;
        let mut reviews = self.read()?.get(scope).reviews;
        for comment in &mut reviews {
            anchor::refresh(&root, comment);
        }
        Ok(reviews)
    }

    fn add_review(&self, scope: &PlanScope, comment: NewReviewComment) -> Result<ReviewComment> {
        let root = self.roots.workspace_root(&scope.workspace)?;
        let captured = anchor::capture(
            &root,
            &comment.path,
            comment.line,
            comment.text_quote.as_deref(),
        )?;
        let now = plan_ops::now_ms();
        let created = ReviewComment {
            id: ReviewId::generate(),
            path: captured.path,
            line: captured.line,
            text_quote: captured.quote,
            line_hash: captured.line_hash,
            file_hash: captured.file_hash,
            body: comment.body,
            state: ReviewState::Draft,
            stale: false,
            created_at: now,
            updated_at: now,
        };
        self.mutate(scope, |data| {
            data.reviews.push(created.clone());
            Ok(created.clone())
        })
    }

    fn update_review(
        &self,
        scope: &PlanScope,
        id: &ReviewId,
        body: String,
    ) -> Result<ReviewComment> {
        self.mutate(scope, |data| {
            let index = Self::find(&mut data.reviews, id)?;
            let comment = &mut data.reviews[index];
            comment.body = body;
            comment.updated_at = plan_ops::now_ms();
            Ok(comment.clone())
        })
    }

    fn resolve_review(&self, scope: &PlanScope, id: &ReviewId) -> Result<ReviewComment> {
        self.mutate(scope, |data| {
            let index = Self::find(&mut data.reviews, id)?;
            let comment = &mut data.reviews[index];
            comment.state = ReviewState::Resolved;
            comment.updated_at = plan_ops::now_ms();
            Ok(comment.clone())
        })
    }

    fn remove_review(&self, scope: &PlanScope, id: &ReviewId) -> Result<()> {
        self.mutate(scope, |data| {
            let index = Self::find(&mut data.reviews, id)?;
            data.reviews.remove(index);
            Ok(())
        })
    }

    fn send_package(&self, scope: &PlanScope) -> Result<ReviewPackage> {
        let root = self.roots.workspace_root(&scope.workspace)?;
        let package = self.mutate(scope, |data| {
            for comment in &mut data.reviews {
                anchor::refresh(&root, comment);
            }
            let open: Vec<&mut ReviewComment> = data.reviews.iter_mut().filter(|c| c.state != ReviewState::Resolved).collect();
            if open.iter().any(|c| c.stale) {
                return Err(Error::conflict("有评论的位置已失效，请先更新或解决这些评论"));
            }
            let mut prompt = String::from("<review-package>\n");
            for comment in &open {
                prompt.push_str(&format!(
                    "  <comment id=\"{}\" path=\"{}\" line=\"{}\">\n    <quote>{}</quote>\n    <body>{}</body>\n  </comment>\n",
                    anchor::xml_escape(comment.id.as_str()),
                    anchor::xml_escape(&comment.path),
                    comment.line,
                    anchor::xml_escape(&comment.text_quote),
                    anchor::xml_escape(&comment.body),
                ));
            }
            prompt.push_str("</review-package>");
            let comment_count = open.len();
            for comment in open {
                comment.state = ReviewState::Sent;
            }
            Ok(ReviewPackage { prompt, comment_count })
        })?;
        debug!(workspace = %scope.workspace, chat = %scope.chat, comment_count = package.comment_count, "review.package.built");
        Ok(package)
    }

    fn step_reviews(&self, scope: &PlanScope) -> Result<StepReviews> {
        Ok(self.read()?.get(scope).steps)
    }

    fn update_reviews(
        &self,
        scope: &PlanScope,
        change: &mut dyn FnMut(&mut Plan, &mut StepReviews) -> Result<()>,
    ) -> Result<(Plan, StepReviews)> {
        self.mutate(scope, |data| {
            change(&mut data.plan, &mut data.steps)?;
            Ok(())
        })?;
        let data = self.read()?.get(scope);
        Ok((data.plan, data.steps))
    }

    fn clear_pending_reviews(&self) -> Result<()> {
        let lock = self.lock()?;
        let mut document = self.read()?;
        let mut cleared = 0;
        for data in document
            .scopes
            .values_mut()
            .flat_map(|chats| chats.values_mut())
        {
            cleared += data.steps.pending.len();
            data.steps.pending.clear();
        }
        if cleared > 0 {
            self.write(&document)?;
            debug!(cleared, "review.pending.cleared");
        }
        drop(lock);
        Ok(())
    }

    fn open_window(
        &self,
        scope: &PlanScope,
        item: &PlanItemId,
        mut baseline: Baseline,
    ) -> Result<Option<Baseline>> {
        let lock = self.lock()?;
        let mut document = self.read()?;
        let mut data = document.get(scope);
        let in_progress = plan_ops::items(&data.plan)
            .any(|i| &i.id == item && i.status == PlanStatus::InProgress);
        if !in_progress || data.steps.baselines.contains_key(item) {
            return Ok(None);
        }
        let others: Vec<&mut ScopeData> = document
            .scopes
            .get_mut(scope.workspace.as_str())
            .into_iter()
            .flat_map(|chats| chats.iter_mut())
            .filter(|(chat, data)| {
                chat.as_str() != scope.chat.as_str() && !data.steps.baselines.is_empty()
            })
            .map(|(_, data)| data)
            .collect();
        if !others.is_empty() {
            baseline.shared = true;
            for other in others {
                for window in other.steps.baselines.values_mut() {
                    window.shared = true;
                }
            }
        }
        data.steps.baselines.insert(item.clone(), baseline.clone());
        document.put(scope, data);
        self.write(&document)?;
        drop(lock);
        debug!(workspace = %scope.workspace, chat = %scope.chat, item = %item, shared = baseline.shared, "review.window.opened");
        Ok(Some(baseline))
    }

    fn windows_elsewhere(&self, scope: &PlanScope) -> Result<bool> {
        Ok(self
            .read()?
            .scopes
            .get(scope.workspace.as_str())
            .is_some_and(|chats| {
                chats.iter().any(|(chat, data)| {
                    chat.as_str() != scope.chat.as_str() && !data.steps.baselines.is_empty()
                })
            }))
    }
}
