//! 模型服务页文案（对标 Aether `providers.*` zh-CN）。

use thinkrail_core::Protocol;

pub const TITLE: &str = "模型服务";
pub const LOADING: &str = "加载中…";
pub const EMPTY: &str = "还没有服务商";
pub const EMPTY_HINT: &str = "新建一个服务商，然后同步模型。";

// 服务商栏
pub const SEARCH: &str = "搜索模型平台…";
pub const FILTER: &str = "筛选服务商";
pub const FILTER_ALL: &str = "全部";
pub const FILTER_ENABLED: &str = "启用";
pub const FILTER_DISABLED: &str = "停用";
pub const NO_MATCHES: &str = "没有匹配的服务商";
pub const ADD: &str = "新建服务商";
pub const EDIT: &str = "编辑";
pub const DELETE: &str = "删除";
pub const ROW_MENU: &str = "更多操作";

// 服务商对话框
pub const NAME: &str = "名称";
pub const NAME_REQUIRED: &str = "请填写名称";
pub const SETTINGS: &str = "服务商设置";
pub const ACCESS: &str = "接入方式";
pub const DIRECT: &str = "直连";
pub const GATEWAY: &str = "网关";
pub const GATEWAY_HELP: &str = "网关：引擎只连本机，真实密钥不进入引擎进程。";
pub const CONFIRM: &str = "确定";
pub const CANCEL: &str = "取消";
pub const SAVE: &str = "保存";
pub const CLOSE: &str = "关闭";
pub const DELETE_PROVIDER: &str = "删除服务商";
pub const RETRY: &str = "重试";
pub const SAVE_FAILED: &str = "保存失败，修改已保留。";

// API 密钥
pub const API_KEY: &str = "API 密钥";
pub const GET_KEY: &str = "获取密钥";
pub const KEY_PLACEHOLDER: &str = "输入新的 API 密钥（留空不修改）";
pub const KEY_SHOW: &str = "显示密钥";
pub const KEY_HIDE: &str = "隐藏密钥";
pub const KEY_COPY: &str = "复制密钥";
pub const KEY_EDIT: &str = "编辑密钥";
pub const KEY_DELETE: &str = "删除密钥";
pub const KEYS_MANAGE: &str = "API 密钥管理";
pub const KEYS_DESC: &str = "管理当前服务商的多个 API 密钥，按顺序轮流使用。";
pub const KEYS_ADD: &str = "添加 API 密钥";
pub const KEYS_EMPTY: &str = "暂无 API 密钥";
pub const KEYS_LABEL: &str = "标签";
pub const KEYS_KEEP: &str = "编辑密钥：输入新值后保存；留空则不修改原密钥。";
pub const KEY_DEFAULT_LABEL: &str = "API Key";
pub const KEY_STRATEGY: &str = "密钥使用方式";
pub const KEY_STRATEGY_ROUND_ROBIN: &str = "轮流使用";
pub const KEY_STRATEGY_ROUND_ROBIN_DESC: &str = "每次请求从下一个密钥开始，把限流分摊到各个密钥";
pub const KEY_EMPTY: &str = "API 密钥不能为空";
pub const CHECK: &str = "检测";
pub const CHECKING: &str = "检测中…";

// API 地址与线路
pub const BASE_URL: &str = "API 地址";
pub const ADD_LINE: &str = "添加线路";
pub const LINE_SETTINGS: &str = "线路设置";
pub const LINE_SETTINGS_TIP: &str = "配置协议、API 地址与自定义请求头";
pub const REMOVE_LINE: &str = "删除线路";
pub const LAST_LINE: &str = "至少保留一条线路。";
pub const PROTOCOL: &str = "协议";
pub const DEFAULT_BADGE: &str = "默认";
pub const SET_DEFAULT_LINE: &str = "设为默认线路";
pub const URL_PLACEHOLDER: &str = "https://api.example.com/v1";
pub const URL_INVALID: &str = "请输入以 http:// 或 https:// 开头的地址";
pub const URL_HINT: &str = "末尾添加 # 可禁用自动附加的 API 版本。";
pub const HEADERS: &str = "自定义请求头";
pub const HEADERS_ADD: &str = "添加请求头";
pub const HEADER_NAME: &str = "请求头名称";
pub const HEADER_VALUE: &str = "请求头取值";
pub const HEADER_INVALID: &str = "请求头名称和取值不能为空。";
pub const HEADERS_SHARED: &str = "请求头对该服务商的所有线路生效。";

// 模型
pub const MODELS: &str = "模型";
pub const NO_MODELS: &str = "未选择模型";
pub const NO_MODEL_MATCHES: &str = "没有匹配的模型";
pub const MODEL_SEARCH: &str = "搜索模型…";
pub const ALL_TYPES: &str = "全部";
pub const CAP_VISION: &str = "视觉";
pub const CAP_REASONING: &str = "推理";
pub const CAP_TOOLS: &str = "工具";
pub const EXPAND_ALL: &str = "全部展开";
pub const COLLAPSE_ALL: &str = "全部折叠";
pub const SYNC_MODELS: &str = "同步模型";
pub const ADD_MODEL: &str = "添加模型";
pub const MODEL_ID: &str = "模型 ID";
pub const REMOVE_GROUP: &str = "移除整组";
pub const REMOVE_MODEL: &str = "移除模型";
pub const MODEL_SETTINGS: &str = "模型设置";
pub const NO_GROUP: &str = "其他";

// 挑选模型
pub const PICKER_TITLE: &str = "挑选模型";
pub const PICKER_HINT: &str = "点 + 添加模型，点 − 移除。";
pub const SEARCH_MODELS: &str = "搜索模型 id";
pub const ADD_ALL: &str = "全部添加";
pub const REMOVE_ALL: &str = "全部移除";
pub const PICKED: &str = "已选";
pub const CAN_VISION: &str = "可看图";
pub const MODEL_TYPE: &str = "类型";

// 模型设置
pub const EDIT_MODEL: &str = "编辑模型";
pub const MODEL_NAME: &str = "显示名称";
pub const MODEL_PROTOCOLS: &str = "线路（首项为默认）";
pub const CONTEXT_WINDOW: &str = "上下文窗口";
pub const MAX_OUTPUT: &str = "最大输出";
pub const CAPABILITIES: &str = "能力";
pub const EFFORTS: &str = "推理档位";
pub const NUMBER_INVALID: &str = "请输入正整数";
pub const MODEL_LINE_REQUIRED: &str = "至少选择一条线路。";
pub const EFFORTS_REQUIRED: &str = "开启推理时至少选择一个档位。";
pub const APPLIES_NEXT: &str = "保存后从下一条消息开始生效；正在生成的回复不受影响。";

pub fn protocol_short(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "OpenAI",
        Protocol::OpenaiResponses => "Responses",
        Protocol::AnthropicMessages => "Anthropic",
    }
}

/// Full protocol name for the protocol dropdown (Aether `RequestConfigDialog`).
pub fn protocol(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "OpenAI 兼容（Chat Completions）",
        Protocol::OpenaiResponses => "OpenAI Responses",
        Protocol::AnthropicMessages => "Anthropic 兼容（Messages）",
    }
}

pub fn model_type(kind: thinkrail_core::ModelType) -> &'static str {
    use thinkrail_core::ModelType as T;
    match kind {
        T::Chat => "对话",
        T::Image => "图像",
        T::Video => "视频",
        T::Tts => "语音合成",
        T::Stt => "语音识别",
        T::Embedding => "向量",
        T::Rerank => "重排",
        T::Unknown => "未知",
    }
}

pub fn probe_get(url: &str) -> String {
    format!("GET {url}")
}
pub fn request_post(url: &str) -> String {
    format!("请求地址：POST {url}")
}
pub fn keys_enabled(enabled: usize, total: usize) -> String {
    format!("{enabled} / {total} 已启用")
}
pub fn more_keys(extra: usize) -> String {
    format!("+{extra}")
}
pub fn check_ok(count: usize) -> String {
    format!("连接正常 · 可用 {count} 个模型")
}
pub fn picked_count(count: usize) -> String {
    format!("已选 {count} 个")
}
pub fn delete_confirm(name: &str) -> String {
    format!("删除 {name}？其模型将从模型选择器中移除，使用过它们的聊天会保留历史记录。")
}
pub fn remove_line_confirm(protocol: Protocol) -> String {
    format!(
        "删除 {} 线路？只在这条线路上的模型会一起删除。",
        protocol_short(protocol)
    )
}
