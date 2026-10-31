//! 提供方设置页文案。
pub const PREVIEW: &str = "预览：";
pub const TITLE: &str = "提供方";
pub const LOADING: &str = "加载中…";
pub const EMPTY: &str = "还没有配置提供方。";
pub const ADD: &str = "添加服务商";
pub const EDIT: &str = "编辑提供方";
pub const NAME: &str = "名称";
pub const API_URL: &str = "API 地址（当前协议）";
pub const URL_HINT: &str = "切换协议只切换当前地址；其他协议地址会保留。末尾添加 # 可禁用自动附加的 API 版本。";
pub const DEFAULT_PROTOCOL: &str = "默认协议";
pub const CHAT: &str = "OpenAI";
pub const RESPONSES: &str = "OpenAI Responses";
pub const ANTHROPIC: &str = "Anthropic";
pub const ACCESS: &str = "接入方式";
pub const DIRECT: &str = "直连";
pub const GATEWAY: &str = "网关";
pub const GATEWAY_HELP: &str = "网关：引擎只连本机，真实密钥不进入引擎进程。";
pub const HEADERS: &str = "额外请求头";
pub const HEADER_NAME: &str = "请求头名称";
pub const HEADER_VALUE: &str = "请求头值";
pub const ADD_HEADER: &str = "添加请求头";
pub const FIND_MODELS: &str = "同步模型";
pub const ADD_MODEL: &str = "手动添加模型 ID";
pub const ADD_BUTTON: &str = "添加";
pub const CHECK: &str = "检测";
pub const KEYS: &str = "API 密钥";
pub const KEYS_MANAGE: &str = "API 密钥管理";
pub const KEYS_DESCRIPTION: &str = "管理当前服务商的多个 API Key";
pub const ADD_KEY: &str = "添加 API 密钥";
pub const KEY_LABEL: &str = "标签";
pub const KEY_VALUE: &str = "密钥";
pub const COPY: &str = "复制";
pub const CLOSE: &str = "关闭";
pub const DELETE: &str = "删除";
pub const ENABLED: &str = "已启用";
pub const CHECK_ALL: &str = "检测所有密钥";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const CONFIRM_DELETE: &str = "确认删除";
pub const SAVING: &str = "保存中…";
pub const PROBING: &str = "检测中…";
pub const ORIGIN_LOCKED: &str = "服务器地址不可更改。要使用其他服务器，请将其添加为新服务器。";
pub const ENABLED_MODELS_REQUIRED: &str = "至少启用一个模型。";
pub const NAME_REQUIRED: &str = "请填写提供方名称。";
pub const URL_REQUIRED: &str = "至少填写一个有效的 API 地址。";
pub const HEADER_INVALID: &str = "请求头名称和值不能为空。";
pub const MODEL_ID_REQUIRED: &str = "模型 ID 不能为空。";
pub const KEY_EMPTY: &str = "API 密钥不能为空。";
pub const VISION: &str = "视觉";
pub const REASONING: &str = "推理";
pub const TOOLS: &str = "工具";
pub const PROTOCOLS: &str = "协议（可多选，首项为默认）";
pub const EFFORTS: &str = "推理档位";
pub const MODEL_NAME: &str = "显示名称";
pub const MODEL_CHECK_OK: &str = "检测通过：{0} ms";
pub const MODEL_CHECK_ERROR: &str = "检测失败：{0}";
pub const KEYS_EMPTY: &str = "API 密钥未配置";
pub const API_KEY_DEFAULT: &str = "API Key";

pub fn model_check_ok(ms: u64) -> String { MODEL_CHECK_OK.replace("{0}", &ms.to_string()) }
pub fn model_check_error(error: &str) -> String { MODEL_CHECK_ERROR.replace("{0}", error) }
pub fn key_summary(enabled: usize, total: usize) -> String { format!("{enabled} / {total} 已启用") }
pub fn model_summary(count: usize) -> String { format!("{count} 个模型") }
pub fn endpoint_preview(value: &str) -> String { format!("{PREVIEW}{value}") }
pub const SEARCH: &str = "搜索服务商…";
