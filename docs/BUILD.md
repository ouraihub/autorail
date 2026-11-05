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
scripts\msvc\cargo-msvc.bat fmt --all                  # 格式化
scripts\msvc\cargo-msvc.bat test --workspace           # 单元测试与集成测试
scripts\msvc\cargo-msvc.bat clippy --workspace --all-targets -- -D warnings
scripts\msvc\cargo-msvc.bat run -p thinkrail-native -- --data-dir D:\tmp\tr-data
```

`rust-toolchain.toml` 固定了 `rustfmt` 和 `clippy` 组件，rustup 会自动安装。GitHub Actions 的 `ci` 检查在每次 push 和 PR 上依次运行 `fmt --all -- --check`、clippy 和 `test --workspace`，用的是和本机相同的命令；推送前先在本机跑一遍。

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

## 换行符

`.gitattributes` 规定：普通文本文件在仓库和工作区都用 LF；`.bat` / `.cmd` 用 CRLF，并且只能是纯 ASCII，否则 cmd.exe 会解析错。`.editorconfig` 让编辑器按同样的规则保存。CI 的 `line-endings` 检查会拒绝仓库里存成 CRLF 的文本文件和含非 ASCII 字符的批处理文件。

这些规则不依赖本机的 `core.autocrlf`。在规则加入之前克隆的仓库，工作区里可能还留着 CRLF 文件；在工作区干净时运行一次：

```powershell
git read-tree HEAD; git checkout-index -f -a
```

## 已知问题

- **debug 构建移动 target 目录后窗口打不开**（`Creating DirectX render pipelines: 0x80070003`）：GPUI 的 debug 构建会从编译时记录的绝对路径读取 shader。重新编译这两个 crate 即可：`cargo clean -p gpui-fast-windows -p gpui-pre-windows`。release 构建会把 shader 编译进二进制，不受影响。

## 运行时要求

应用安装包不带 Agent 引擎。打开“设置 → 编码搭档”：已在 `PATH` 上的 `pi` / `omp` / `dsh` 会被检测到并标“系统”；没装的点“安装”，应用下载固定版本的 Node v24.22.2 或 Bun 1.4.4（校验 SHA-256），再把 pi 1.4.0、OMP 18.14.2、DSH 0.2.1-rc.5 装到 `<data>/coding`，标“内置”。“设为默认”决定新聊天用哪个引擎；新建项目或新建聊天时没装引擎、没设默认，界面会给出引导按钮直接打开这一页。引擎使用自己的默认配置目录（pi `~/.pi/agent`、OMP `~/.omp/agent`、DSH `~/.dsh`）。中转地址、模型和 API 密钥在“模型服务”中配置；每个引擎用哪个服务商在“编码搭档”里选，也可以选“{引擎} 官方”继续用引擎原有的模型配置。

模型、提供方、密钥、MCP、默认 Agent 和聊天绑定存在 `<exe>/data/thinkrail.db`；显式 `--data-dir <目录>` 时数据库在该目录。项目与日志默认位于 `%USERPROFILE%/.autorail`，不读取或自动迁移 `.thinkrail`、`.thinkrail-native` 的数据；旧开发版的 `agent-profiles.json` 不再读取，旧版本号的 `thinkrail.db` 需要删除后重建。API 密钥按用户选择明文存储，复制/显示才取出；请保护数据库文件。所有项目共用 OMP 的默认配置目录，MCP 集合以最后启动的项目为准。

模型选择器使用本机检出的 LumaKit `crates/lumakit-model-picker` 路径依赖（见 `crates/app/Cargo.toml`），不依赖 LumaKit 的 AI／数据库／设计系统。移动检出目录时同步调整该路径。入口位于聊天输入框下方，浮层向上展开；顶部不再保留模型和力度的重复下拉。图标为原 ThinkRail 使用的 Remix 4.9.0 SVG，许可保存在共享 crate 的 `LICENSE-REMIX`。

2026-11-05 验证：workspace 280 项通过，原有 16 项下载／外部引擎测试保持 ignored；fmt 与严格 clippy 通过。隔离 `--data-dir` 和 `USERPROFILE`、真实 pi 会话中操作了分组、附加 high 的搜索、力度点击／拖动／键盘、默认值存储、重置 off、刷新和 Esc。未发送模型请求，不冒充上游回答验证。
