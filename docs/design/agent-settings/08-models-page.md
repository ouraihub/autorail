

### Native implementation status (2026-11-07, R7/R8 integration)

Implemented in the native tree:

- `ReviewPrefs`, model-page review controls, and review settings persistence.
- `RouteRequest.model/effort` overrides with clamping and pinned-model routing errors.
- `PlanItem.changes`, `StepReviews`, review prompt/fact-package text, verdict parsing and §5.6 state transitions.
- `WorkspaceService` Git window operations and serialized work-window reconciliation/host commits.
- `HostToolOptions`, conditional `request_review`, reviewer-finding resolution, MCP worker-thread cancellation, and the 1200-second pi MCP timeout.
- `SessionRole::Reviewer` launch restrictions for pi, OMP, and DSH, including read-only tool allowlists and DSH permission automation.
- App-level `AppReviewRunner`: single-flight reviewer sessions, model/effort routing, private reviewer sink, completion timeout, verdict application, router/session/support cleanup, and `ReviewEvent` delivery.
- Workbench review-event pumping and TODO row review-status projection: running, approved, changes requested, and failed.

The R7/R8 contract is implemented in the preceding section. Remaining validation is real-engine smoke:
the reviewer runtime must be exercised with `THINKRAIL_SMOKE_PI`, `THINKRAIL_SMOKE_OMP`, and
`THINKRAIL_SMOKE_DSH`; ignored smoke tests remain intentional when those variables are absent.

Verification completed for the implemented layers: workspace-wide `cargo check --workspace --all-targets`; focused tests for `thinkrail-agents`, `thinkrail-host-tools`, and `thinkrail-plan-review` (63, 15, 14 integration/spec, and 26 tests respectively, all passing).

## R7/R8 Native integration contract

### R7 ReviewRunner

`crates/app/src/services/review.rs` owns the only native `ReviewRunner` implementation. It is
constructed by `services::build` and injected into `HostTools::with_reviews`; `host-tools` never
starts engine sessions itself.

The runner is keyed by `(workspace, chat, item)` and stores one `ReviewHandle` per in-flight item.
`start` is single-flight: an existing item returns the existing handle with `joined: true`; a new
item inserts a handle before spawning background work. `cancel(scope)` cancels every in-flight
review in that chat and finishes its handle as `Failed`.

Each background review:

1. Reads the worker chat binding, workspace root, project, backend and current `ReviewPrefs`.
2. Chooses `review.model`, otherwise the worker's default model, otherwise engine default. A pinned
   review model that cannot route is an error; effort is `review.effort` or the default effort and
   is clamped by routing.
3. Allocates a fresh reviewer `ChatId`, support directory and `RouteRequest` with `new_session`.
4. Starts the selected runtime with `SessionRole::Reviewer { system_prompt }`, no `resume`, no host
   tools, the worker cwd and the review routing.
5. Uses a private `SessionSink` to observe snapshots/messages. Completion is the first transition
   from `Running` to `Idle`; an engine error or a 15-minute deadline is failure. The last assistant
   text is parsed with `plan_review::verdict::parse`.
6. Reads the plan and `StepReviews.pending[item]`, applies the parsed result with
   `plan_review::verdict::apply`, and renders worker text with `review::result_text`.
7. Always disposes the reviewer session, releases its router chat, removes its support directory,
   and removes the in-flight entry. Cleanup is best effort after the primary result is captured.

`ReviewEvent` is sent for UI-visible outcomes. Synchronous pi/OMP tool callers consume the returned
`ReviewHandle` and do not receive a duplicate prompt. Button, auto-re-review and async DSH outcomes
send `ReviewEvent::Finished`; DSH and fix instructions use the worker chat's pending prompt queue.

### R7 event and pending prompt channels

`services::Channels` gains `reviews: Receiver<ReviewEvent>`. `ChatPanel` receives it through a
dedicated Workbench pump. `Chat` owns `pending_prompts: VecDeque<String>` and `review_cards` keyed
by `PlanItemId`.

When a chat is idle, the first pending prompt is sent through `AgentSession::prompt`; when running,
it remains queued. A closed chat keeps the queue and sends it after resume. The fixed DSH prefix is
`【评审结论】` and is not included in the worker-facing synchronous tool result.

### R8 status projection

The TODO strip does not duplicate review state into `PlanItem`. It reads `StepReviews` and projects:

- `pending[item]` → `正在评审…`;
- latest record with open findings → `需要修复 · N`;
- latest approved record with no open findings → `已检查`;
- runner failure → `评审失败`;
- `Done` with `ItemChange` and no record/pending → `检查` action.

The strip's `检查` action calls the same runner with `ReviewOrigin::Button`; it is not gated by
`ReviewPrefs.agent_review`. A `ReviewCard` contains item id/title, running/outcome, summary and
findings. Findings are expandable and use existing `rf_…` ids. `request_changes` with a spent
cycle exposes `发给 Agent 修复`; failure exposes `重试`. Both actions use the pending prompt queue.

### R7/R8 acceptance tests

- A fake runtime/session proves single-flight, completion parsing, verdict persistence, cleanup on
  success/failure/cancel, and no duplicate synchronous prompt.
- A Workbench channel test proves a DSH result queues while running and sends once after idle.
- Plan-row projection tests cover running, open findings count, approved, failed, and check states.
- Workspace-wide `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` remain required. Real-engine smoke tests are separate and stay ignored
  unless the corresponding `THINKRAIL_SMOKE_*` executable variables are set.
