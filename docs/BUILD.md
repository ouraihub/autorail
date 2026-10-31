# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.1，依赖已精确锁定在 `Cargo.lock` 中。

```powershell
cargo build                      # 开发构建
cargo test --workspace           # 单元测试
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p thinkrail-native -- --data-dir D:\tmp\tr-data
```

需要真实进程的测试默认标为 ignored，按需运行：

```powershell
cargo test -p thinkrail-terminal windows_pty_lifecycle_smoke -- --ignored --nocapture
# 引擎握手：先设置 THINKRAIL_AGENT_EXE / THINKRAIL_AGENT_ARGS（JSON 数组）/ THINKRAIL_AGENT_BACKEND / THINKRAIL_AGENT_CWD
cargo test -p thinkrail-agents real_official_engine_handshake -- --ignored --nocapture
```

## Windows：没有安装 MSVC 时

如果已安装 Visual Studio Build Tools（含“使用 C++ 的桌面开发”），直接构建即可。

如果没有安装，可以用 LLVM 加 [xwin](https://github.com/Jake-Shadle/xwin) 获取的 Windows SDK 构建。这种方式不修改系统配置，也不需要写入 `.cargo/config.toml`。

```powershell
xwin --accept-license --manifest-version 18 splat --disable-symlinks --output C:\sdk\xwin
$llvm = "C:\path\to\llvm\bin"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "$llvm\lld-link.exe"
$env:CC = "$llvm\clang-cl.exe"; $env:CXX = $env:CC
$env:AR = "$llvm\llvm-lib.exe"; $env:RC = "$llvm\llvm-rc.exe"
$env:LIB = "C:\sdk\xwin\crt\lib\x86_64;C:\sdk\xwin\sdk\lib\um\x86_64;C:\sdk\xwin\sdk\lib\ucrt\x86_64"
$env:INCLUDE = "C:\sdk\xwin\crt\include;C:\sdk\xwin\sdk\include\ucrt;C:\sdk\xwin\sdk\include\um;C:\sdk\xwin\sdk\include\shared;C:\sdk\xwin\sdk\include\winrt"
```

注意：PATH 中如果有 Git for Windows 的 `usr\bin`，其中的 `link.exe` 是 GNU coreutils 的 link 命令，不是链接器。必须像上面那样显式指定链接器。

## 已知问题

- **debug 构建移动 target 目录后窗口打不开**（`Creating DirectX render pipelines: 0x80070003`）：GPUI 的 debug 构建会从编译时记录的绝对路径读取 shader。重新编译这两个 crate 即可：`cargo clean -p gpui-fast-windows -p gpui-pre-windows`。release 构建会把 shader 编译进二进制，不受影响。

## 运行时要求

应用不内置任何 Agent 引擎。在“设置 → Agents”中，为每个档案填写可执行文件的绝对路径和启动参数：

| 后端 | 可执行文件 | 参数示例 |
|---|---|---|
| pi | `node.exe`（Node ≥ 22.19） | `["C:/…/@earendil-works/pi-coding-agent/dist/bundle/cli.js"]` |
| OMP | `bun.exe`（Bun ≥ 1.4.4） | `["C:/…/@oh-my-pi/pi-coding-agent/src/cli.ts"]`，并且配置目录中要有可用模型 |
| DSH | `node.exe`（Node ≥ 24.9） | `["C:/…/@deepseek-ai/dsh/lib/bin.js"]`（预发布版） |
