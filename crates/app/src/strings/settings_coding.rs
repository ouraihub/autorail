//! 编码搭档页文案（对标 Cherry Studio `code.*` zh-CN）。

use thinkrail_core::BackendKind;

pub const TITLE: &str = "编码搭档";
pub const NOT_INSTALLED: &str = "未安装";
pub const INSTALL: &str = "安装";
pub const INSTALLING: &str = "安装中…";
pub const INSTALLING_HINT: &str = "首次安装需要下载运行时，耗时数分钟";
pub const RETRY: &str = "重试";
pub const SYSTEM: &str = "系统";
pub const MANAGED: &str = "内置";
pub const SET_DEFAULT: &str = "设为默认";
pub const IS_DEFAULT: &str = "默认";
pub const PROVIDERS: &str = "模型服务商";
pub const SEARCH: &str = "搜索服务商…";
pub const NO_MATCHES: &str = "没有匹配的服务商";
pub const GATEWAY: &str = "统一网关";
pub const GATEWAY_DESC: &str = "全部服务商经本机网关接入，真实密钥不进入引擎进程";
pub const OFFICIAL_DESC: &str = "使用引擎自己配置目录里的模型与登录";
pub const ENABLE: &str = "启用";
pub const ENABLED: &str = "已启用";
pub const ALL_PROVIDERS: &str = "未选择：使用全部已启用的服务商，各按自己的接入方式";
pub const ADD_PROVIDER: &str = "在 设置 → 模型服务 添加服务商";

pub fn official(backend: BackendKind) -> String { format!("{} 官方", name(backend)) }
pub fn version(version: &str) -> String { format!("版本 {version}") }

/// Display name (Cherry's `code.cli_tools.*` where it has one).
pub fn name(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Pi => "Pi",
        BackendKind::Omp => "Oh My Pi",
        BackendKind::Dsh => "DeepSeek Harness",
    }
}
