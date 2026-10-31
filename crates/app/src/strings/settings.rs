//! Strings of the settings dialog.

pub const TITLE: &str = "设置";
pub const AGENTS: &str = "Agent 档案";
pub const NEW: &str = "新建档案";
pub const EDIT: &str = "编辑";
pub const DELETE: &str = "删除";
pub const CONFIRM_DELETE: &str = "确认删除";
pub const SET_DEFAULT: &str = "设为默认";
pub const DEFAULT: &str = "默认";
pub const PRERELEASE: &str = "预发布";
pub const ENABLED: &str = "已启用";
pub const DISABLED: &str = "已禁用";
pub const ENGINE_DEFAULT_DIR: &str = "引擎默认配置目录";
pub const REVISION: &str = "版本";
pub const EMPTY: &str = "还没有 Agent 档案。新建一个后，新聊天会默认使用它。";

pub const FORM_NEW: &str = "新建 Agent 档案";
pub const FORM_EDIT: &str = "编辑 Agent 档案";
pub const BACKEND: &str = "后端";
pub const LABEL: &str = "名称";
pub const EXECUTABLE: &str = "可执行文件（绝对路径）";
pub const ARGS: &str = "启动参数（JSON 字符串数组）";
pub const CONFIG_ROOT: &str = "配置目录（绝对路径，可留空）";
pub const ENV_REFS: &str = "环境变量引用（每行一个：变量名=启动环境中的变量名）";
pub const STATE: &str = "状态";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const KEEP_REVISION: &str = "已有聊天继续使用版本";
pub const HOST_FLAGS: &str = "不要填写 --mode、--session、--profile、--approval-mode 等参数，应用会自动追加。";

pub const EXAMPLE_PI: &str = "示例：node.exe，参数 [\"C:\\\\…\\\\pi-coding-agent\\\\dist\\\\bundle\\\\cli.js\"]";
pub const EXAMPLE_OMP: &str = "示例：omp.exe，参数 []；或 bun.exe，参数 [\"C:\\\\…\\\\pi-coding-agent\\\\src\\\\cli.ts\"]";
pub const EXAMPLE_DSH: &str = "示例：node.exe（≥ 24.9），参数 [\"C:\\\\…\\\\dsh\\\\lib\\\\bin.js\"]";

pub const ERR_LABEL: &str = "请填写档案名称";
pub const ERR_EXECUTABLE: &str = "可执行文件必须是绝对路径";
pub const ERR_ARGS: &str = "启动参数必须是 JSON 字符串数组，例如 [\"C:\\\\tools\\\\cli.js\"]";
pub const ERR_CONFIG_ROOT: &str = "配置目录必须是绝对路径，或者留空";
pub const ERR_ENV: &str = "环境变量引用每行必须是 名称=变量名";
