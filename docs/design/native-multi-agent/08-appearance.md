# 08 外观：设置 → 界面 → 外观

状态：**已实现**（2026-11-03）。沿用本设计包的分层、错误与日志规范和事实标记。

目标：设置左栏在 AGENT 分组下新增“界面”分组和“外观”页，完整移植原版 ThinkRail 的外观设置：主题（模式、主题、对比度、跟随系统、自定义主题文件）和字体（预览、界面与代码字体、字重、代码字号、连字、备用字体、自定义字体文件）。改动立即生效，不需要保存按钮，重启后保留。

## 1. 原版的做法（源码，`thinkrail` 仓库）

| 方面 | 原版行为 | 位置 |
|---|---|---|
| 页面结构 | “主题”组 → “字体”组（预览卡、界面、代码、高级）→ “自定义字体” → “自定义主题” → “编辑器”（行宽） | `apps/web/src/panels/AppearanceSettings.tsx:342-480` |
| 主题模式 | 分段：系统 / 浅色 / 深色。选浅色或深色时，在同系列里换到对应外观的主题（`appearanceTarget`）；选系统时写入 `themeMode: system` 和浅深配对 | 同上 `:93-104,296-304` |
| 主题 | 固定模式：可展开的列表，每行一个主题，当前项打勾。系统模式：“本设备当前值”一行（系统外观 → 主题）和“主题组合”展开项（浅色主题、深色主题两个下拉框） | 同上 `:160-203,382-451` |
| 对比度 | 分段：默认 / 高。在同系列、同外观里换到对应对比度的主题（`counterpart`，按 id 去掉 `-light`/`-dark` 和 `high-contrast-` 前缀后比较） | 同上 `:71-91,317-329` |
| 不可用提示 | 配置的主题已不存在（例如删掉了自定义主题文件）时显示警告：“所配置的主题不可用，已改用 X。” | 同上 `:333-381` |
| 主题解析 | 固定模式用 `theme`，找不到回落到 `dark`。系统模式按系统外观取配对里的槽位，槽位不存在或外观不符时，取该外观里内置主题中 order 最小的普通对比度主题。首次进入系统模式时：当前主题占自己的槽位，另一个槽位取对比度相同的内置主题 | `apps/web/src/themes/runtime.ts:295-417` |
| 内置主题 | 6 个：Dark(10)、Light(20)、Minimal Dark(30)、Minimal Light(40)、High Contrast Dark(50)、High Contrast Light(60)；列表顺序是默认主题 `dark` 在前，其余按 order、label、id | `themes/bundled/*.theme.json`；`runtime.ts:164-175` |
| 自定义主题 | `<数据目录>/themes/*.theme.json`，每秒轮询目录变化；每个文件最多 256 KB，最多 64 个；目录 id 加 `user.` 前缀；文件坏了逐个报原因，不影响其他文件。页面显示说明、文件夹路径（复制、打开）和问题列表 | `packages/server/src/themes/SPEC.md`；`apps/web/src/themes/schema.ts`；`panels/CustomThemesSettings.tsx` |
| 字体设置 | `interfaceFont`、`codeFont`（`{family: 内置 \| 名称 \| null, fallbacks: 最多 8 个}`）、`codeFontSize`（10–24，默认 13；编辑器和 diff 比它小 2）、`codeLigatures`（默认关）、`fontWeightOffset`（-70–130，步长 10，默认 0） | `packages/contracts/src/domain.ts:1261-1343,1460-1471` |
| 字体组 | 预览卡（对话文字、行内代码、代码块、两个按钮、“编辑器和 diff · N px”、“终端 · N px”），界面：字体、字重；代码：字体、字号（− 输入 +）、连字；高级：两组备用字体。每个改过的控件旁有“恢复默认” | `panels/FontsSettings.tsx:99-161,344-591` |
| 字体选择器 | 分组列出内置、自定义、本机已安装的字体，可搜索，也可以直接输入任何字体名；代码字体只列等宽字体 | `panels/FontPicker.tsx` |
| 自定义字体 | `<数据目录>/fonts/`，.ttf/.otf/.woff/.woff2，每个最多 50 MB，最多 128 个；可拖入或选择文件上传；同名替换；显示每个文件的字体名和“使用中” | `packages/server/src/fonts/SPEC.md`；`panels/CustomFontsSettings.tsx` |
| 生效 | 每次改动立即发 `settings.update`，失败时提示“无法更改主题”并保持原值；所有窗口和浏览器同时更新 | `AppearanceSettings.tsx:287-294` |

原版的字体组没有中文翻译（`zh-CN.json` 里没有 `settings.appearance.fonts.*`），所以截图里显示英文。

## 2. 原生版现状（源码）

- 设置对话框只有一个“AGENT”分组，`Page` 枚举里有 5 个页面（`crates/app/src/panels/settings/mod.rs:31-37,191-200`）。
- 只内置了 `dark`、`light` 两个主题（`crates/ui/themes/`，与原版文件一致）。启动时由 `--light` 参数二选一，运行中不能切换（`crates/app/src/main.rs:44-53,111`）。
- 字体和字号都是常量：`metrics::text::*` 一共 174 处调用，都经过 `components::TextStyled::text_style` 这一个入口。终端直接用 `fonts::CODE` 和 `text::CODE_TEXT`，单元格宽度在创建时按 `字号 × 0.6` 估算（`panels/terminal.rs:96-112`）。
- 偏好存在 `thinkrail.db` 的 `app_prefs` 表里（键值对），目前只有日志页的开关和日志级别（`crates/settings-store/src/app_prefs.rs`）。
- GPUI 提供了需要的能力：`window.appearance()` 和 `observe_window_appearance`（系统深浅色）、`text_system().all_font_names()`（本机字体）、`add_fonts`（运行中注册字体文件）、`FontFallbacks`、`FontFeatures`、`refresh_windows`（重绘所有窗口）、`prompt_for_paths`（选择文件）、`reveal_path`（在资源管理器中显示）。
- DirectWrite 的限制（**源码**，实现时读到）：不显式设为 0 时，`liga`、`clig`、`calt` 都默认打开，所以改动前的原生版其实一直在画 JetBrains Mono 的连字，而原版默认 `font-variant-ligatures: none`；`FontFallbacks` 只在系统已安装的字体里找，注册进来的自定义字体不能当备用字体；首选字体不存在时，不会用 `Font.fallbacks`，而是用 GPUI 自带的回落列表。

## 3. 决定

| 编号 | 决定 |
|---|---|
| A1 | 设置左栏改成两组：“AGENT”（原有 5 页）和“界面”（外观）。原版的“会话、语言、布局、Keyboard、终端”和“系统”组在原生版没有对应功能，这次不加。 |
| A2 | 外观设置是一个整体：`core::AppearancePrefs`，作为一个 JSON 值存到 `app_prefs` 的 `appearance` 键里，不改表结构。读取时逐个字段校验，不合法的字段用默认值，不影响其他字段（同原版 `normalizeFontSettings`）。 |
| A3 | 字段与原版 `AppConfig` 一一对应：`theme`、`theme_mode`（fixed / system）、`system_pair`（light、dark）、`interface_font`、`code_font`（`family: Builtin \| Named(String) \| None`，`fallbacks` 最多 8 个）、`code_font_size`（10–24，默认 13）、`code_ligatures`（默认 false）、`font_weight_offset`（-70–130，步长 10，默认 0）。 |
| A4 | 6 个内置主题全部带上。主题目录、解析规则、配对、对比度切换、系列匹配都放在 `crates/ui` 的 `theme/catalog.rs`，逻辑逐条按第 1 节移植。主题文件按原版 v2 格式解析，`dangerHover` / `onDanger` 可以省略，按原版规则推导（`danger` 的各通道乘以 0.88；`text`、`background` 中对两种底色对比度更高的那个）。 |
| A5 | 自定义主题：`<数据目录>/themes/*.theme.json`，限制与原版相同（256 KB、64 个、嵌套 32 层）；id 加 `user.` 前缀；文件坏了逐个显示原因。每秒检查一次目录（名称、大小、修改时间），有变化就重新读取并刷新。回落和推导出的另一个配对槽位只从内置主题里选。 |
| A6 | 跟随系统：用 GPUI 的 `window.appearance()`；系统切换深浅色时，经 `observe_window_appearance` 重新解析并重绘。 |
| A7 | 应用方式：`ThinkrailTheme::install` 接收解析后的主题；所有视图在渲染时读全局主题和字体设置，改动后调用 `refresh_windows`，所有窗口立即更新。启动时在打开窗口前读一次偏好，第一帧就是用户选的主题，不会先闪一下默认主题。 |
| A8 | `--light` 启动参数删除：主题由外观设置决定。（多 Agent 设计包没有记录这个参数。） |
| A9 | 字体在运行中生效：`crates/ui` 新增 `Typography`（界面字体、代码字体、备用字体、代码字号、连字、字重偏移），`text_style` 从它取字体名、字重和代码字号，174 处调用方都不用改。`text_style` 拿不到 `App`，所以 `Typography` 存在一个进程级的 `RwLock` 里，只由 UI 线程写入。各代码样式保持和默认 13 px 的差值：`CODE_TEXT`、`CODE_DOCUMENT` 等于代码字号，`CODE_SMALL` 等于代码字号减 2，默认设置下界面不变。品牌字体 Orbitron 不受影响。 |
| A10 | 字重偏移加到每个样式的字重上。内置字体是静态字重文件（Geist 300/400/500/600，JetBrains Mono 400/500），偏移后取最接近的那个字重，所以只能分档变化；页面在内置字体下显示原版的“只有几个固定字重，滑块会分档改变它”提示。 |
| A11 | 终端在字体或字号变化时重新量单元格宽度和行高（用 GPUI 量 `M` 的宽度，代替原来的 `字号 × 0.6`），然后重新计算行列数并通知 PTY；终端永远不画连字（同原版）。 |
| A12 | 字体选择器：一个可搜索的弹窗，分组列出“内置”“自定义”“已安装”，最后一项“使用‘输入的名称’”。代码字体只列等宽字体。等宽判断：内置和自定义字体读文件里的 `post.isFixedPitch` 和 PANOSE；已安装字体比较 `i` 和 `M` 的宽度。选的字体在本机不存在时，保留选择，同时提示“X 在这台设备上不可用，正在使用 Y。” |
| A13 | 自定义字体：`<数据目录>/fonts/`，限制与原版相同（50 MB、128 个）。DirectWrite 只能加载 .ttf / .otf，所以原生版只接受这两种（原版还接受 .woff / .woff2，这是有意的偏离）；.ttc / .otc 字体集合报“不支持字体集合”。字体名、是否等宽、字重用 `ttf-parser` 读取（它已在依赖图里，`Cargo.lock` 0.25.1，作为直接依赖加入时使用精确版本）。“选择文件…”按钮打开系统文件选择框，把文件复制到字体目录：先写到同目录下的临时文件，校验通过后再改名，校验失败不会覆盖原有文件。目录每秒检查一次变化，新文件经 `add_fonts` 注册。GPUI 不能注销已注册的字体，所以删掉的文件在重启前仍可使用；页面上会说明这一点。 |
| A14 | 不做“编辑器 → 行宽”：原生版的文件查看器还没有行宽设置（原版是 `settings.lineWidth.file`）。 |
| A15 | 文案：主题部分用原版的中文；原版字体部分只有英文，这次按项目规范翻译成中文。 |
| A16 | 模块边界：`crates/ui` 仍不依赖 `core`，也不做文件 I/O：它只解析传进来的内容（主题 JSON、字体字节），解析函数接收普通参数（主题 id、系统配对、系统外观）。偏好类型在 `core`，读写目录、复制文件、轮询、把偏好换算成 `ui` 的参数都在 `crates/app`。 |
| A17 | 连字默认关闭：代码样式默认把 `liga`、`clig`、`calt` 都设为 0，与原版一致。这改变了原生版之前的默认显示（`->`、`!=` 不再连在一起）。 |
| A18 | 选的字体不存在时，由 `crates/app` 换成该用途的内置字体再交给 `Typography`，并显示 A12 的“不可用”提示；自定义字体只能作为首选字体，不能当备用字体，选择器的备用字体列表里不列出它们。 |

## 4. 契约变化（设计）

```rust
// core（app_prefs.rs）
pub enum ThemeMode { Fixed, System }
pub struct SystemThemePair { pub light: String, pub dark: String }
pub enum FontFamily { Builtin, Named(String) }      // Builtin = 该用途的内置字体
pub struct FontChoice { pub family: Option<FontFamily>, pub fallbacks: Vec<FontFamily> }
pub struct AppearancePrefs {
    pub theme: String, pub theme_mode: ThemeMode, pub system_pair: Option<SystemThemePair>,
    pub interface_font: FontChoice, pub code_font: FontChoice,
    pub code_font_size: u8, pub code_ligatures: bool, pub font_weight_offset: i16,
}
// AppPrefs: + appearance
// AppPrefService: + fn set_appearance(&self, appearance: &AppearancePrefs) -> Result<AppPrefs>;
```

`crates/ui` 公开：主题目录（内置 + 自定义）、`resolve(固定主题 id, 系统配对与当前系统外观) -> Resolution { theme, fallback }`、`counterpart`、`appearance_target`、`derive_pair`、单个主题文件的解析；`Typography` 和 `fonts::inspect(bytes)`（字体名、是否等宽、字重）。`crates/ui` 不访问文件和数据库（A16）。

## 5. 模块落点

| 位置 | 内容 |
|---|---|
| `crates/core/src/appearance.rs`（新） | 上面的类型和校验（按字段回落默认值）；`app_prefs.rs` 只增加 `appearance` 字段和 `set_appearance` |
| `crates/settings-store/src/app_prefs.rs` | `appearance` 键的读写 |
| `crates/ui/src/theme/manifest.rs`（新） | v2 主题文件解析（从字符串）、可推导字段、问题码 |
| `crates/ui/src/theme/catalog.rs`（新） | 内置 + 自定义目录、排序、解析、配对、系列与对比度匹配 |
| `crates/ui/src/typography.rs`（新） | `Typography`、样式到字体的映射、字重偏移、GPUI Kit 组件字体 |
| `crates/ui/src/fonts/inspect.rs`（新） | 用 `ttf-parser` 读取字体文件的字体名、是否等宽、字重 |
| `crates/app/src/appearance.rs` 与 `files`、`select`、`typeset`、`watch` 子模块（新） | 全局状态、启动时应用、写偏好并刷新窗口；读目录与复制字体；主题模式/对比度/配对的选择规则；偏好到 `Typography`；系统外观监听和每秒检查目录 |
| `crates/app/src/panels/settings/appearance.rs` 与子模块（新） | 外观页：主题组、字体组（预览、选择器）、自定义字体、自定义主题 |
| `crates/app/src/panels/settings/mod.rs` | 新增“界面”分组和外观页 |
| `crates/app/src/panels/terminal*.rs` | 字体变化时重新量单元格 |
| `crates/app/src/main.rs` | 删 `--light`，启动时按偏好安装主题和字体 |

## 6. 验证

- 单元测试（**实测**通过）：偏好校验（每个字段越界或类型不对时回落默认值，其余字段保留）；主题文件解析（缺键、多余键、错误颜色、可推导字段、大小和数量限制、`user.` 前缀、重复 id）；解析规则（固定模式回落、系统槽位外观不符、首次配对、对比度切换、系列匹配、只从内置主题回落）；选择规则（浅色/深色留在同系列、进入系统模式推导配对并保留、高对比度配对、单独设置槽位）；偏好到字体（大小写不敏感、缺失字体换成内置字体、自定义字体不能当备用字体）；自定义字体与主题目录（.ttc 拒绝、大小和数量限制、复制失败不覆盖原文件、变化签名）。
- 无界面窗口测试（**实测**通过，`panels/settings/appearance/tests.rs`）：点“浅色”后全局主题变成 `light`，对比度点“高”后变成 `high-contrast-light`，代码字号点“+”后 `Typography` 变成 14 px，三项都写进了数据库；字体选择器打开时按 Escape 只关闭选择器（修复前会关掉整个设置对话框）。
- 真实窗口（**实测**，2026-11-03，窗口不抢前台，消息直接发给应用窗口）：重启后保留上次的模式和代码字号；浅色、高对比度浅色、切回深色立即生效；系统模式按 Windows 当前浅色显示浅色主题，浅色/深色槽位可单独选，恢复默认有效；界面字体搜索并选用 Bahnschrift 后预览立即变化，恢复默认有效；连字开关只影响编辑器和预览，终端不变；字号 +/− 与预览卡“编辑器和 DIFF 小 2 px”一致；放入 `themes/` 的有效文件 1 秒内出现在列表并可选用，坏文件显示解析错误；删掉正在用的自定义主题后显示“所配置的主题不可用，已改用 深色。”；放入 `fonts/` 的 .ttf 出现在自定义字体里，.woff 显示不支持的原因。
- 未在真实窗口实测：系统深浅色在运行中切换（会改用户的系统设置）、“选择文件…”与“打开文件夹”（会弹出系统对话框和资源管理器）。前者由 `observe_window_appearance` 驱动，选择逻辑有单元测试覆盖。
