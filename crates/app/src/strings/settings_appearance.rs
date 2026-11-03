//! 外观页文案（08 A15）：主题部分沿用原版中文；原版字体与自定义部分只有英文，这里按原文翻译。

pub const GROUP: &str = "界面";
pub const TITLE: &str = "外观";

pub const THEME_GROUP: &str = "主题";
pub const THEME_MODE: &str = "主题模式";
pub const MODE_SYSTEM: &str = "系统";
pub const MODE_LIGHT: &str = "浅色";
pub const MODE_DARK: &str = "深色";
pub const THEME: &str = "主题";
pub const CONTRAST: &str = "对比度";
pub const CONTRAST_DEFAULT: &str = "默认";
pub const CONTRAST_HIGH: &str = "高";
pub const CURRENT_ON_DEVICE: &str = "本设备当前值";
pub const THEME_PAIR: &str = "主题组合";
pub const LIGHT_THEME: &str = "浅色主题";
pub const DARK_THEME: &str = "深色主题";
pub const CHANGE_FAILED: &str = "无法更改主题";

pub fn fallback_fixed(theme: &str) -> String {
    format!("所配置的主题不可用，已改用 {theme}。")
}
pub fn fallback_light(theme: &str) -> String {
    format!("所配置的浅色主题不可用。正在使用 {theme}。")
}
pub fn fallback_dark(theme: &str) -> String {
    format!("所配置的深色主题不可用。正在使用 {theme}。")
}

/// The original's localized names of the bundled themes; user themes show their own label.
pub fn bundled_theme_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "dark" => "深色",
        "light" => "浅色",
        "high-contrast-dark" => "高对比度深色",
        "high-contrast-light" => "高对比度浅色",
        "minimal-dark" => "极简深色",
        "minimal-light" => "极简浅色",
        _ => return None,
    })
}

pub const FONTS_TITLE: &str = "字体";
pub const FONTS_DESCRIPTION: &str = "改动立即应用到 ThinkRail 的所有窗口。";
pub const PREVIEW_TITLE: &str = "重构设置对话框";
pub const PREVIEW_META: &str = "main · 3 个文件 · 2 分钟前";
pub const PREVIEW_BEFORE_BOLD: &str = "我把字体令牌集中到了一处，并更新了";
pub const PREVIEW_BOLD: &str = "三个文件";
pub const PREVIEW_BEFORE_CODE: &str = "。运行 ";
pub const PREVIEW_CODE: &str = "bun run test";
pub const PREVIEW_AFTER_CODE: &str = " 检查——全部通过。";
pub const PREVIEW_OPEN_PR: &str = "打开拉取请求";
pub const PREVIEW_REVIEW: &str = "审查改动";
pub fn preview_editor(size: u8) -> String {
    format!("编辑器和 diff · {size} px")
}
pub fn preview_terminal(size: u8) -> String {
    format!("终端 · {size} px")
}

pub const INTERFACE_TITLE: &str = "界面";
pub const INTERFACE_DESCRIPTION: &str = "菜单、会话和文档。";
pub const CODE_TITLE: &str = "代码";
pub const CODE_DESCRIPTION: &str = "编辑器、diff、代码块和终端。";
pub const FONT: &str = "字体";
pub const RESET_INTERFACE_FONT: &str = "把界面字体恢复为默认";
pub const RESET_CODE_FONT: &str = "把代码字体恢复为默认";
pub fn unavailable(family: &str, fallback: &str) -> String {
    format!(
        "{family} 在这台设备上不可用，正在使用 {fallback}。你的选择会保留，字体可用时自动恢复。"
    )
}
pub const WEIGHT: &str = "字重";
pub const WEIGHT_DESCRIPTION: &str = "同时调整界面和代码中的所有字重，标题仍然比正文粗。";
pub const WEIGHT_LIGHTER: &str = "更细";
pub const WEIGHT_HEAVIER: &str = "更粗";
pub const RESET_WEIGHT: &str = "把字重恢复为默认";
pub fn weight_steps(family: &str) -> String {
    format!("{family} 只有几个固定字重，这个滑块会分档改变它。")
}
pub const SIZE: &str = "字号";
pub fn size_description(delta: u8) -> String {
    format!("终端和代码块。编辑器和 diff 比它小 {delta} px。")
}
pub const SMALLER: &str = "减小";
pub const LARGER: &str = "增大";
pub const RESET_SIZE: &str = "把代码字号恢复为默认";
pub const LIGATURES: &str = "连字";
pub const LIGATURES_DESCRIPTION: &str =
    "代码字体支持时，把 => 和 != 画成一个符号。终端从不绘制连字。";
pub const RESET_LIGATURES: &str = "把连字恢复为默认";
pub const ADVANCED: &str = "高级";
pub const ADVANCED_SUMMARY: &str = "备用字体";
pub const INTERFACE_FALLBACKS: &str = "界面备用字体";
pub const INTERFACE_FALLBACKS_DESCRIPTION: &str = "用于界面字体没有的字符。";
pub const CODE_FALLBACKS: &str = "代码备用字体";
pub const CODE_FALLBACKS_DESCRIPTION: &str = "用于代码字体没有的符号，例如终端提示符图标。";
pub const ADD_FALLBACK: &str = "添加备用字体";
pub const FONTS_CHANGE_FAILED: &str = "无法更改字体设置";

pub const PICKER_SEARCH: &str = "搜索，或输入任意字体名";
pub const PICKER_BUILTIN: &str = "内置";
pub const PICKER_CUSTOM: &str = "自定义";
pub const PICKER_INSTALLED: &str = "已安装";
pub const PICKER_MONO: &str = "等宽";
pub const PICKER_MONO_ONLY: &str = "代码字体只列出等宽字体。";
pub const PICKER_EMPTY: &str = "没有匹配的字体。";
pub fn picker_use(family: &str) -> String {
    format!("使用“{family}”")
}

pub const CUSTOM_FONTS_TITLE: &str = "自定义字体";
pub const CUSTOM_FONTS_DESCRIPTION: &str = "这个文件夹里的字体不用安装就能在 ThinkRail 中使用。";
pub const CUSTOM_FONTS_CHOOSE: &str = "选择文件…";
pub const CUSTOM_FONTS_HINT: &str = ".ttf 或 .otf。同名文件会替换旧文件。";
pub const CUSTOM_FONTS_FOLDER: &str = "字体文件夹";
pub const CUSTOM_FONTS_IN_USE: &str = "使用中";
pub const CUSTOM_FONTS_REMOVE_HINT: &str =
    "要移除字体，删除这个文件夹里的文件；已加载的字体在重启前仍可使用。";
pub fn custom_fonts_added(families: &str) -> String {
    format!("已添加 {families}。")
}
pub const FONT_INVALID: &str = "这个文件不是有效的字体。";
pub const FONT_TOO_LARGE: &str = "文件大于 50 MB。";
pub const FONT_TOO_MANY: &str = "只读取前 128 个字体文件。";
pub const FONT_UNREADABLE: &str = "无法读取这个文件。";
pub const FONT_COLLECTION: &str = "不支持字体集合（.ttc），请改用单独的 .otf 或 .ttf 文件。";
pub const FONT_UNSUPPORTED: &str = "不支持 .woff 和 .woff2，请改用 .otf 或 .ttf 文件。";

pub const CUSTOM_THEMES_TITLE: &str = "自定义主题";
pub const CUSTOM_THEMES_STEPS: &str = "复制内置的 dark.theme.json，改一个新的 id 和 label，修改颜色，然后以 .theme.json 结尾的文件名（例如 ocean.theme.json）保存到下面的文件夹。保存后，你的主题会立即出现在上面的列表里。";
pub const CUSTOM_THEMES_FOLDER: &str = "主题文件夹";
pub const THEME_INVALID_JSON: &str = "这个文件不是有效的 JSON。";
pub const THEME_INVALID: &str = "这个文件不是完整的主题。";
pub const THEME_ID_TOO_LONG: &str = "主题 id 太长，最多 59 个字符。";
pub const THEME_DUPLICATE_ID: &str = "另一个主题文件已经用了这个 id。";
pub const THEME_TOO_LARGE: &str = "文件大于 256 KB。";
pub const THEME_TOO_MANY: &str = "只读取前 64 个主题文件。";
pub const THEME_UNREADABLE: &str = "无法读取这个文件。";

pub const COPY_PATH: &str = "复制路径";
pub const OPEN_FOLDER: &str = "打开文件夹";
pub const PATH_COPIED: &str = "已复制文件夹路径";

pub const EDITOR_TITLE: &str = "编辑器";
pub const LINE_WIDTH: &str = "行宽";
pub const LINE_WIDTH_DESCRIPTION: &str = "文件与 diff 中每行的最大字符数。";
pub const LINE_WIDTH_INPUT: &str = "文件行宽";
pub const FIT_TO_PANE: &str = "适应面板";
pub const FIT_TO_PANE_DESCRIPTION: &str = "面板较窄时用更短的行宽折行。";
pub const FIT_TO_PANE_SWITCH: &str = "文件行宽适应面板";
pub const LINE_WIDTH_FAILED: &str = "无法更改文件行宽";
pub const RESET_LINE_WIDTH: &str = "把行宽恢复为默认";
pub const RESET_FIT_TO_PANE: &str = "把适应面板恢复为默认";
pub fn line_width_invalid(min: u16, max: u16) -> String {
    format!("请输入 {min} 到 {max} 之间的整数。")
}
