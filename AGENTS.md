# ThinkRail Native：AI 协作入口

开始任何编码任务前，必须读取 docs/AI工程规范.md。
它约束架构边界、接口、模块、日志、错误、语言、测试和验证。
项目局部规则只能扩展它，不得降低安全、错误处理和验证要求。

## 按需读取

- `SPEC.md`：crate 分层、依赖方向、模块与测试组织，以及本项目对规范的补充。改动模块边界、公开入口或测试位置前必读。
- `docs/BUILD.md`：所有 cargo 命令必须经 `scripts\msvc\cargo-msvc.bat` 执行；直接运行 `cargo` 会用错链接器。
- `docs/design/native-multi-agent/`：core 契约、错误码、日志、持久化与验收。
- `docs/design/agent-settings/`：模型、提供方、MCP 与默认 Agent。

## 验证命令

在仓库根目录运行，交付前全部通过：

```powershell
scripts\msvc\cargo-msvc.bat fmt --all -- --check
scripts\msvc\cargo-msvc.bat clippy --workspace --all-targets -- -D warnings
scripts\msvc\cargo-msvc.bat test --workspace
```

只编译通过不能证明界面或后端行为；涉及界面的改动要在真实 GPUI 窗口中操作验证，验证方式见 `SPEC.md` 的 Verification。

## LumaKit 依赖

`lumakit-model-picker`、`lumakit-usage-status` 在根 `Cargo.toml` 以 Git 依赖固定到 `https://github.com/ouraihub/LumaKit.git` 的完整 40 位提交号（`rev`），`crates/app` 用 `.workspace = true` 引用。

- 不得改成 `path` 依赖或指向本机检出目录：CI 和其他机器没有这个目录，读取 workspace 时就会失败。
- 需要 LumaKit 的新改动时：
  1. 先在 LumaKit 仓库提交并推送到 GitHub，记下完整提交号；未推送的提交不能引用。
  2. 把根 `Cargo.toml` 两处 `rev` 改成这个提交号（两处必须相同）。
  3. 运行 `scripts\msvc\cargo-msvc.bat metadata --format-version 1` 更新锁文件；不要用 `cargo update`，它可能顺带升级无关依赖。确认 `Cargo.lock` 里只有 LumaKit 来源及其确实改变的依赖有变化。
  4. 跑上面三条验证命令；涉及界面的改动按 `SPEC.md` 的 Verification 实窗验证。
- 联调 LumaKit 尚未推送的改动时，只能在本机临时加 `[patch."https://github.com/ouraihub/LumaKit.git"]` 指向本地检出；提交前删掉该 patch，并还原随之变化的 `Cargo.lock`。
- LumaKit 与本仓库使用的 `gpui-kit` 版本必须一致；升级 GPUI Kit 时两边一起升，再按第 1～4 步更新 `rev`。
