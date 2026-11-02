pub const TITLE: &str = "项目";
pub const OPEN: &str = "打开项目";
pub const CLOSE: &str = "关闭项目";
pub const NEW_WORKSPACE: &str = "新建工作区";
pub const REMOVE_WORKSPACE: &str = "删除工作区";
pub const EMPTY: &str = "还没有项目。点击 + 选择一个文件夹。";
pub const DEFAULT_WORKSPACE: &str = "默认";
pub const INIT_TITLE: &str = "初始化 Git 仓库";
pub const INIT: &str = "初始化";
pub const BRAND: &str = "ThinkRail";
pub const OPEN_SUBTITLE: &str = "选择一个本地文件夹或 Git 仓库开始工作。";
pub const CREATE_TITLE: &str = "新建项目";
pub const CREATE_SUBTITLE: &str = "创建一个空仓库，再和 Agent 一起开始写代码。";
pub const CREATE_DESC: &str = "ThinkRail 会在这台电脑上创建文件夹，并在里面初始化一个 Git 仓库。";
pub const CREATE_NAME: &str = "名称";
pub const CREATE_NAME_PLACEHOLDER: &str = "weather-app";
pub const CREATE_LOCATION: &str = "位置";
pub const CREATE_PATH_HINT: &str = "填写名称后显示项目文件夹路径。";
pub const ERR_NAME: &str = "请填写项目名称。";
pub const ERR_LOCATION: &str = "请填写项目位置。";
pub const BROWSE: &str = "浏览";
pub const CREATE: &str = "创建并打开";
pub const CREATING: &str = "创建中…";
pub const CANCEL: &str = "取消";
pub const SETUP_TITLE: &str = "让 Agent 起草规格";
pub const SETUP_DETAIL: &str = "打开一个聊天，运行「设置项目」技能，从项目目标开始起草规格。";

pub fn init_body(path: &std::path::Path) -> String {
    format!(
        "{} 还不是 Git 仓库。是否在这里执行 git init（分支 main）并打开为项目？",
        path.display()
    )
}
