# ThinkRail Native

A Rust/GPUI desktop workbench with in-process Rust application services. The user selects pi, OMP or DSH through explicit configuration profiles; chats keep their original profile/revision binding. Each external engine owns its model loop, native history and tools.

## Modules

A Cargo workspace of single-purpose crates. `core` holds the contracts; every service crate implements one trait from it and depends only on `core`; the app holds trait objects and is the only place concrete services are built (`crates/app/src/services.rs`).

- `core`: domain types, service traits, error model and session events. No I/O, no UI.
- `workspace` / `profiles` / `plan-review` / `terminal`: projects, worktrees, Git and contained file reads; revisioned Agent profiles and chat bindings; plans and anchored review comments; real PTYs.
- `agents`: official pi / OMP JSONL and DSH ACP subprocess adapters, one `AgentRuntime` per backend. Node/Bun may run the selected external engine; neither runs application business services.
- `host-tools`: the TODO / review tool core over `plan-review`'s plan semantics, the host's authenticated named pipe, and the stdio MCP server behind `thinkrail-native mcp-host`. pi and DSH reach it through that subcommand, OMP in-process through `set_host_tools`.
- `ui`: visual tokens taken from the original ThinkRail themes, bundled fonts and stateless components. No services.
- `app`: composition root, logging and the GPUI panels. Panels talk to services only through `AppState` and background jobs.

Service crates never depend on each other, except `host-tools` → `plan-review` for the shared plan operations. Engine adapters never call each other. Design detail, decisions and the acceptance matrix live in `docs/design/native-multi-agent`.

The host is the only writer of application metadata. Engine native history is not converted into application state or rewritten across engines. Client layout belongs to each window. A new default never changes an existing chat's backend.

Source application services are ported only when used by a reachable feature. No browser application, legacy desktop carrier, website or unrelated publishing pipeline is copied into this project. Migration is based on observable behavior, not retained file count.

Direct dependencies use exact latest non-yanked stable releases where compatible; pre-release versions and Git snapshots are not silently substituted. Rust is pinned to 1.99.1, GPUI Kit to the crates.io 0.8.1 release with GPUI Fast, and the dependency graph is committed in Cargo.lock. Upgrades require a Windows native-window smoke and exercised protocol behavior, not only dependency resolution.

## Verification

Use disposable project/config/data roots. Unit tests exercise filesystem, Git, request correlation and state transitions. Smoke tests launch the actual host/SDK processes; native verification opens a real GPUI window and exercises input and output. A successful build alone is not a UI or backend behavior proof.
