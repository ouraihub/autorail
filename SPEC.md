# ThinkRail Native

A Rust/GPUI desktop workbench with in-process Rust application services. The user picks pi, OMP or DSH as the default Agent on the 编码搭档 page (installed by the app or found on `PATH`); each chat stays bound to the backend it was created with. Each external engine owns its model loop, native history and tools.

## Modules

A Cargo workspace of single-purpose crates. `core` holds the contracts; service crates implement its traits and depend on `core`; the app holds trait objects and is the only place concrete services are built (`crates/app/src/services.rs`).

- `core`: domain types, service traits, error model and session events. No I/O, no UI.
- `workspace` / `plan-review` / `terminal`: projects, worktrees, Git and contained file reads; plans and anchored review comments; real PTYs.
- `settings-store`: one Turso database for global provider endpoints/models/keys, model preferences (including the default Agent), chat bindings and global/project MCP configuration. Provider writes are revisioned; secret values are never serialized into engine support files.
- `routing`: prepares typed engine projections, read-only launch signatures, the authenticated loopback gateway and provider probes. Depends on `core` only; stores are injected through traits.
- `agents`: official pi / OMP JSONL and DSH ACP subprocess adapters, one `AgentRuntime` per backend. Node/Bun may run the selected external engine; neither runs application business services. Every launch stages the original ThinkRail workflow skills (`crates/agents/skills`, from `pi-thinkrail-workflow`, `spec-graph` and `pi-todos`) and loads them through the engine's own skill path (pi `--skill`, OMP `skills.customDirectories`, DSH `skill-filesystem.customSkillDirs`), plus the original `WORKFLOW_RULE` / `SPEC_RULE` system-prompt sections. The engine expands skill commands itself: `/skill:<name>` for pi and OMP, `/<name>` for DSH.
- `host-tools`: the TODO / review and `spec_*` tool core over `plan-review`'s plan semantics and the workspace's spec files, the host's authenticated named pipe, and the stdio MCP server behind `thinkrail-native mcp-host`. pi and DSH reach it through that subcommand, OMP in-process through `set_host_tools`.
- `ui`: visual tokens taken from the original ThinkRail themes, bundled fonts and stateless components. No services.
- `observability`: the tracing subscriber, daily Beijing-time log files, the runtime log level, the per-action `op{id at}` span and the bounded tail the 后台日志 page reads.
- `app`: composition root and the GPUI panels. Panels talk to services only through `AppState` and background jobs; `state::run` opens one `op` span per action.

Service crates never depend on each other, except `host-tools` → `plan-review` for the shared plan operations. Engine adapters never call each other. Design detail, decisions and the acceptance matrix live in `docs/design/native-multi-agent`.

## Rust module and test organization

[`docs/AI工程规范.md`](docs/AI工程规范.md) is the baseline for every change. The rules below only extend it for this repository; where a rule here is stricter, the stricter rule applies. They apply to new code and to every production file substantially modified.

### File responsibility and size

- Every `.rs` file MUST have one primary responsibility stated by its module comment or owning `SPEC.md`. Closely related private helpers MAY remain in the same file; unrelated UI, persistence, network, and domain concerns MUST NOT.
- A production file above 300 physical lines SHOULD be reviewed for responsibility before more code is added. A production file MUST NOT exceed 500 physical lines unless the change description records why extraction would make the boundary worse; generated, vendored, and fixture files are exempt.
- An existing file above 400 lines MUST NOT grow during an unrelated change. A feature change that crosses 400 lines MUST split the file in the same change, not defer the split.
- Line count is a review trigger, not the design. Split by independent state, dependency direction, lifecycle, or public contract—not by arbitrary chunks or one-function files.

### Directory modules and public surface

- A meaningful feature directory MUST use `mod.rs` as its coordinator and boundary. Sibling modules MUST be private by default; expose only the smallest `pub(super)` or `pub(crate)` surface required by the coordinator.
- The coordinator owns composition, routing between submodules, and feature-level state. It MUST NOT also own a second implementation of a submodule's persistence, probe, CRUD, or row-rendering logic.
- A submodule MUST NOT reach into another submodule's private fields. Pass an intent-specific method or narrow data type instead. Do not solve a boundary problem with `pub` fields, duplicate state, re-exports, or a second implementation.

`crates/app/src/panels/settings/providers/` is the concrete example: `mod.rs` coordinates `rail.rs` (provider list), `detail.rs` (provider detail), `keys.rs`, `models.rs` and `dialogs.rs`.

### Tests

Follows AI 工程规范 §8 and §10, which match the Rust community convention:

- **Unit tests** live in `src` next to the code they test, in a `#[cfg(test)] mod tests` block; when the block grows, move it to a sibling `<module>/tests.rs` or `<module>_tests.rs` registered with `#[cfg(test)] mod tests;`. Unit tests MAY use private items, but production code MUST NOT be made public only to be reachable from a test.
- **Integration tests** live in the crate's `tests/` directory and use only the public API. Shared helpers go in `tests/common/mod.rs`, never in a `tests/common.rs` without test functions.
- Production modules MUST NOT depend on test-only code, fixtures, or helpers.
- Tests MUST cover observable behavior, state transitions, errors and boundaries at the narrowest owning level. Do not add tests that pin layout text, forwarding copies, wiring, or incidental defaults.
- A test file MAY exceed the ordinary production-file target when its fixtures are inherently large, but it still MUST remain organized by contract and SHOULD be split before 500 lines.

`clippy.toml` allows `unwrap`, `expect` and printing inside `#[test]` functions only. Integration test crates also put helpers outside `#[test]` functions, so each file under `tests/` opens with `#![allow(clippy::unwrap_used, clippy::expect_used, reason = "…")]`. That exemption MUST NOT appear in `src`.

### Required pre-edit review

Before adding a non-trivial feature to a Rust module, the author MUST:

1. list the file's current responsibilities and physical line count;
2. identify the state owner, service boundary, and every affected caller;
3. decide whether the change belongs in the coordinator or a submodule;
4. place unit tests beside the code and integration tests under the crate's `tests/`;
5. run the affected check after the split, not only before it.

The implementation MUST finish with no stale duplicate path, compatibility copy, or unused extracted module.

### Match arms over enums

A `match` on an enum this workspace owns MUST name every variant, so adding a variant is a compile error at every decision point instead of a silent fallthrough. A bare `_ =>` stays only where it is justified:

- the scrutinee is a third-party or open type (`serde_json::Value`, `serde_yaml::Value`, `turso::Error`, `std::io::ErrorKind`), protocol strings or numbers;
- the arm binds and passes the value through unchanged (`other => …`, `error => error`), so every variant takes the same path by design.

Check with `cargo-msvc.bat clippy --workspace --lib --bins -- -W clippy::wildcard_enum_match_arm`; every remaining warning must fall under one of the two cases above. The lint is not enabled in the gate because it also flags those justified arms.

### Status against the baseline

As of 2026-11-02 no production file exceeds 500 lines. The 15 files that did were split by responsibility into child modules next to their coordinators, moving code without changing behavior. Eleven files are between 400 and 500 lines; they fall under the growth rule in "File responsibility and size" and are split when a feature change would push them past 400.

The host is the only writer of application metadata. Engine native history is not converted into application state or rewritten across engines. Client layout belongs to each window. A new default never changes an existing chat's backend.

Settings are global across engines; effective MCP configuration is project-scoped. Default model preferences and the default Agent apply only to new chats. Configuration changes dispose idle engines (or defer until idle), preserve their native binding and restart only on the next send. Engines use their own default config directories; per-chat support files and gateway tokens are cleaned up after disposal, before another launch can use the same support directory. DSH user routes are merged without rewriting user YAML; OMP merges only host-owned `thinkrail-` entries in `mcp.json`.

Source application services are ported only when used by a reachable feature. No browser application, legacy desktop carrier, website or unrelated publishing pipeline is copied into this project. Migration is based on observable behavior, not retained file count.

Direct dependencies use exact latest non-yanked stable releases where compatible; pre-release versions and Git snapshots are not silently substituted. Rust is pinned to 1.99.1, GPUI Kit to the crates.io 0.8.1 release with GPUI Fast, and the dependency graph is committed in Cargo.lock. Upgrades require a Windows native-window smoke and exercised protocol behavior, not only dependency resolution.

## Verification

Use disposable project/config/data roots. Unit tests exercise filesystem, Git, request correlation and state transitions. Smoke tests launch the actual host/SDK processes; native verification opens a real GPUI window and exercises input and output. A successful build alone is not a UI or backend behavior proof.
