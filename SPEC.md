# ThinkRail Native

A Rust/GPUI desktop workbench with in-process Rust application services. The user selects pi, OMP or DSH through explicit configuration profiles; chats keep their original profile/revision binding. Each external engine owns its model loop, native history and tools.

## Modules

A Cargo workspace of single-purpose crates. `core` holds the contracts; service crates implement its traits and depend on `core`; the app holds trait objects and is the only place concrete services are built (`crates/app/src/services.rs`).

- `core`: domain types, service traits, error model and session events. No I/O, no UI.
- `workspace` / `profiles` / `plan-review` / `terminal`: projects, worktrees, Git and contained file reads; revisioned Agent profiles and chat bindings; plans and anchored review comments; real PTYs.
- `settings-store`: one Turso database for global provider endpoints/models/keys, model preferences and global/project MCP configuration. Provider writes are revisioned; secret values are never serialized into engine support files.
- `routing`: prepares typed engine projections, read-only launch signatures, the authenticated loopback gateway and provider probes. Depends on `core` only; stores are injected through traits.
- `agents`: official pi / OMP JSONL and DSH ACP subprocess adapters, one `AgentRuntime` per backend. Node/Bun may run the selected external engine; neither runs application business services.
- `host-tools`: the TODO / review tool core over `plan-review`'s plan semantics, the host's authenticated named pipe, and the stdio MCP server behind `thinkrail-native mcp-host`. pi and DSH reach it through that subcommand, OMP in-process through `set_host_tools`.
- `ui`: visual tokens taken from the original ThinkRail themes, bundled fonts and stateless components. No services.
- `app`: composition root, logging and the GPUI panels. Panels talk to services only through `AppState` and background jobs.

Service crates never depend on each other, except `host-tools` → `plan-review` for the shared plan operations. Engine adapters never call each other. Design detail, decisions and the acceptance matrix live in `docs/design/native-multi-agent`.

## Rust module and test organization

These rules apply to new code and to every production file substantially modified. They prevent UI panels and services from becoming unreviewable god files; they are repository policy, not a claim about an upstream Rust language limit.

### File responsibility and size

- Every `.rs` file MUST have one primary responsibility stated by its module comment or owning `SPEC.md`. Closely related private helpers MAY remain in the same file; unrelated UI, persistence, network, and domain concerns MUST NOT.
- A file SHOULD stay at or below 400 physical lines. Crossing 400 lines REQUIRES a responsibility review before more code is added. A production file MUST NOT exceed 500 physical lines unless the change description records why extraction would make the boundary worse; generated, vendored, and fixture files are exempt.
- An existing file above 400 lines MUST NOT grow during an unrelated change. A feature change that crosses 400 lines MUST split the file in the same change, not defer the split.
- Line count is a review trigger, not the design. Split by independent state, dependency direction, lifecycle, or public contract—not by arbitrary chunks or one-function files.

### Directory modules and public surface

- A meaningful feature directory MUST use `mod.rs` as its coordinator and boundary. Sibling modules MUST be private by default; expose only the smallest `pub(super)` or `pub(crate)` surface required by the coordinator.
- The coordinator owns composition, routing between submodules, and feature-level state. It MUST NOT also own a second implementation of a submodule's persistence, probe, CRUD, or row-rendering logic.
- A submodule MUST NOT reach into another submodule's private fields. Pass an intent-specific method or narrow data type instead. Do not solve a boundary problem with `pub` fields, duplicate state, re-exports, or a second implementation.
- For a feature with multiple concerns, the expected shape is:

  ```text
  feature/
    mod.rs       coordinator and public boundary
    draft.rs     pure draft/validation transformations
    keys.rs      key state and key operations
    models.rs    model state and model operations
    tests.rs     tests for the feature's pure/state contracts
  ```

  `providers/` follows this shape: `mod.rs`, `draft.rs`, `keys.rs`, `models.rs`, and `tests.rs`. This is an example of responsibility boundaries, not a mandatory filename template for every feature.

### Test and production separation

- New feature tests MUST live in a sibling `tests.rs` or `*_tests.rs` module under the owning feature directory. Production files MUST NOT accumulate large inline `#[cfg(test)]` modules.
- Test modules MAY access the owning module's private implementation through `#[cfg(test)] mod tests;`, but production modules MUST NOT depend on test-only code, fixtures, or helpers.
- Tests MUST cover pure transformations and observable state transitions at the narrowest owning boundary. Do not add tests that pin layout text, forwarding copies, wiring, or incidental defaults.
- A test file MAY exceed the ordinary production-file target when its fixtures are inherently large, but it still MUST remain organized by contract and SHOULD be split before 500 lines.

### Required pre-edit review

Before adding a non-trivial feature to a Rust module, the author MUST:

1. list the file's current responsibilities and physical line count;
2. identify the state owner, service boundary, and every affected caller;
3. decide whether the change belongs in the coordinator or a submodule;
4. place new tests in the separate test module;
5. run the affected check after the split, not only before it.

The implementation MUST finish with no stale duplicate path, compatibility copy, unused extracted module, or test code left in the production coordinator.

The host is the only writer of application metadata. Engine native history is not converted into application state or rewritten across engines. Client layout belongs to each window. A new default never changes an existing chat's backend.

Settings are global across engine profiles; effective MCP configuration is project-scoped. Default model preferences apply only to new sessions. Configuration changes dispose idle engines (or defer until idle), preserve their native binding and restart only on the next send. Per-chat support files and gateway tokens are cleaned up after disposal, before another launch can use the same support directory. DSH user routes are merged without rewriting user YAML; OMP merges only host-owned `thinkrail-` entries in `mcp.json`.

Source application services are ported only when used by a reachable feature. No browser application, legacy desktop carrier, website or unrelated publishing pipeline is copied into this project. Migration is based on observable behavior, not retained file count.

Direct dependencies use exact latest non-yanked stable releases where compatible; pre-release versions and Git snapshots are not silently substituted. Rust is pinned to 1.99.1, GPUI Kit to the crates.io 0.8.1 release with GPUI Fast, and the dependency graph is committed in Cargo.lock. Upgrades require a Windows native-window smoke and exercised protocol behavior, not only dependency resolution.

## Verification

Use disposable project/config/data roots. Unit tests exercise filesystem, Git, request correlation and state transitions. Smoke tests launch the actual host/SDK processes; native verification opens a real GPUI window and exercises input and output. A successful build alone is not a UI or backend behavior proof.
