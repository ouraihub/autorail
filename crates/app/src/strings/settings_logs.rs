//! 后台日志设置页文案（native-multi-agent 06 §5）。

pub const TITLE: &str = "后台日志";
pub const SHOW: &str = "显示后台日志";
pub const SHOW_HINT: &str = "日志只在此页打开时读取，不影响后台写入。";
pub const LEVEL: &str = "日志级别";
pub const LEVEL_HINT: &str = "调试级别记录每次操作的完整调用链，排查问题时打开；立即生效。";
pub const LEVEL_PINNED: &str = "已由环境变量 THINKRAIL_LOG 指定，此处设置不生效。";
pub const INFO: &str = "信息";
pub const DEBUG: &str = "调试";
pub const ONLY_PROBLEMS: &str = "只看问题";
pub const REFRESH: &str = "刷新";
pub const OPEN_DIR: &str = "打开日志目录";
pub const COPY: &str = "复制当前日志";
pub const COPY_TRACE: &str = "复制";
pub const COPIED: &str = "已复制当前显示的日志";
pub const FILTER: &str = "筛选：Trace ID、op、chat、模块名";
pub const EVENT_MENU: &str = "按事件筛选";
pub const LAYER_MENU: &str = "按层级筛选";
pub const ALL_EVENTS: &str = "全部事件";
pub const ALL_LAYERS: &str = "全部层级";
pub const EMPTY: &str = "还没有日志。";
pub const NO_PROBLEMS: &str = "没有警告或错误。";
pub const NO_MATCHES: &str = "没有符合筛选的日志。";
pub const READ_FAILED: &str = "日志读取失败：";

pub fn recent(count: usize) -> String {
    format!("最近 {count} 条 · 每 2 秒刷新")
}

/// Tooltip and accessible name of a row's short TraceId tag.
pub fn filter_trace(short: &str) -> String {
    format!("只看 Trace {short}… 的日志；再点一次取消筛选")
}

/// The 事件 menu's name for a lifecycle event.
pub fn event(code: &str) -> &'static str {
    match code {
        "ENTRY" => "开始",
        "EXIT" => "完成",
        "ERROR" => "失败",
        "CANCEL" => "取消",
        "TIMEOUT" => "超时",
        "UNFINISHED" => "未结束",
        _ => "其他",
    }
}
