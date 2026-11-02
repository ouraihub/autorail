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
