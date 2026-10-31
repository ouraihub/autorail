# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.1，依赖已精确锁定在 `Cargo.lock` 中。

Windows 上统一用便携 MSVC（MSVC 14.44 + Windows SDK 26100，x64）构建，即使已经装了 Visual Studio Build Tools 也一样。脚本在 `scripts/msvc/`，取自 [`@ouraihub/desktop-kit`](https://github.com/ouraihub/desktop-kit)（MIT，许可见 `scripts/msvc/LICENSE`），与 AutoNote 共用同一套工具链。

首次使用，在仓库根运行一次（下载约 455 MB 的微软构建组件，需要 `uv` 和 `curl`）：

```powershell
scripts\msvc\setup-msvc.bat
```

默认安装到 `D:\Applications\msvc-portable\msvc`，整台机器只需一份。装到别处时设置 `PORTABLE_MSVC_ROOT` 指向含 `setup_x64.bat` 的目录。

之后所有 cargo 命令都经 `cargo-msvc.bat` 执行，参数原样传给 cargo：

```powershell
scripts\msvc\cargo-msvc.bat build                      # 开发构建
scripts\msvc\cargo-msvc.bat test --workspace           # 单元测试
scripts\msvc\cargo-msvc.bat clippy --workspace --all-targets -- -D warnings
scripts\msvc\cargo-msvc.bat run -p thinkrail-native -- --data-dir D:\tmp\tr-data
```

## 打包 EXE

从仓库根目录运行：

```powershell
scripts\msvc\package-exe.bat
```

脚本会使用便携 MSVC 构建 `--workspace --release`，然后生成：

```text
dist\thinkrail-native\thinkrail-native.exe
dist\thinkrail-native-x64.zip
```

也可以传版本号，让 ZIP 带版本名：

```powershell
scripts\msvc\package-exe.bat 0.1.0
```

发布包不内置用户数据；首次运行默认使用 `%USERPROFILE%\.autorail`，也可通过 `--data-dir <目录>` 隔离数据。

需要真实进程的测试默认标为 ignored，按需运行：

```powershell
scripts\msvc\cargo-msvc.bat test -p thinkrail-terminal windows_pty_lifecycle_smoke -- --ignored --nocapture
# 隔离配置、占位模型，无凭据、不发模型请求。按实际安装位置设置：
$env:THINKRAIL_SMOKE_PI = 'C:/path/node.exe|C:/path/pi-coding-agent/dist/bundle/cli.js'
$env:THINKRAIL_SMOKE_OMP = 'C:/path/omp.exe'
$env:THINKRAIL_SMOKE_DSH = 'C:/path/node.exe|C:/path/dsh/lib/bin.js'
scripts\msvc\cargo-msvc.bat test -p thinkrail-agents smoke:: -- --ignored --nocapture --test-threads=1
```

它先注入便携 MSVC 的 `PATH`、`LIB`、`INCLUDE`，并检查 `cl.exe`、`link.exe` 确实来自便携目录，再调用 cargo。直接运行 `cargo` 会用到 Git for Windows `usr\bin` 里的 `link.exe`（GNU coreutils 的 link 命令，不是链接器），报 `extra operand … Try 'link --help'`。Windows SDK 的 `rc.exe` 也来自这套工具链，`turso` 等依赖的构建脚本需要它。

`.bat` 文件必须保持 CRLF 和纯 ASCII（`.gitattributes` 已固定），否则 cmd.exe 会解析错。

## 已知问题

- **debug 构建移动 target 目录后窗口打不开**（`Creating DirectX render pipelines: 0x80070003`）：GPUI 的 debug 构建会从编译时记录的绝对路径读取 shader。重新编译这两个 crate 即可：`cargo clean -p gpui-fast-windows -p gpui-pre-windows`。release 构建会把 shader 编译进二进制，不受影响。

## 运行时要求

应用不内置任何 Agent 引擎。在“设置 → Agent 档案”中，为每个档案填写可执行文件的绝对路径和启动参数；在“提供方”中配置中转地址、模型及 API 密钥，或继续使用引擎原有模型配置。

| 后端 | 可执行文件 | 参数示例 |
|---|---|---|
| pi | `node.exe`（Node ≥ 22.19） | `["C:/…/@earendil-works/pi-coding-agent/dist/bundle/cli.js"]` |
| OMP | `omp.exe` 或 `bun.exe`（Bun ≥ 1.4.4） | 可执行发行版参数 `[]`；源码入口 `["C:/…/@oh-my-pi/pi-coding-agent/src/cli.ts"]`。使用宿主 MCP 时填写独立配置目录 |
| DSH | `node.exe`（Node ≥ 24.9） | `["C:/…/@deepseek-ai/dsh/lib/bin.js"]`（预发布版） |

模型、提供方、密钥和 MCP 存在 `<exe>/data/thinkrail.db`；显式 `--data-dir <目录>` 时数据库在该目录。项目、Agent 档案与日志默认位于 `%USERPROFILE%/.autorail`，不读取或自动迁移 `.thinkrail`、`.thinkrail-native` 的数据。API 密钥按用户选择明文存储，复制/显示才取出；请保护数据库文件。不同项目共用同一个 OMP 配置目录时，MCP 集合以最后启动的项目为准，建议使用独立档案目录。
