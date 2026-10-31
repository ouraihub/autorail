//! ThinkRail Native desktop app: composition root, logging and GPUI views.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logging;
mod panels;
mod services;
mod state;
mod strings;
mod workbench;

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use gpui_kit::{AppContext as _, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use thinkrail_ui::{Appearance, ThinkrailTheme, UiAssets, fonts};
use tracing::{error, info};

struct Args {
    data_dir: PathBuf,
    appearance: Appearance,
}

fn args() -> anyhow::Result<Args> {
    let mut data_dir = None;
    let mut appearance = Appearance::Dark;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => data_dir = Some(PathBuf::from(args.next().context("--data-dir 需要一个目录")?)),
            Some("--light") => appearance = Appearance::Light,
            _ => bail!("未知参数：{}（可用：--data-dir <目录>、--light）", arg.to_string_lossy()),
        }
    }
    let data_dir = match data_dir {
        Some(dir) => dir,
        None => PathBuf::from(std::env::var_os("USERPROFILE").context("找不到用户目录，请用 --data-dir 指定数据目录")?)
            .join(".thinkrail-native"),
    };
    Ok(Args { data_dir, appearance })
}

fn main() {
    // An engine launches this same executable as its stdio MCP server; no window, no logging.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "mcp-host") {
        if let Err(error) = thinkrail_host_tools::run_mcp_stdio() {
            use std::io::Write as _;
            let _ = writeln!(std::io::stderr(), "{error}");
            std::process::exit(1);
        }
        return;
    }
    let args = match args() {
        Ok(args) => args,
        Err(error) => fail(&format!("{error:#}")),
    };
    let _log = match logging::init(&args.data_dir) {
        Ok(guard) => guard,
        Err(error) => fail(&format!("{error:#}")),
    };
    info!(version = env!("CARGO_PKG_VERSION"), "app starting");
    let (services, channels) = match services::build(&args.data_dir) {
        Ok(built) => built,
        Err(error) => {
            error!(error = %format!("{error:#}"), "startup failed");
            fail(&format!("{error:#}"))
        }
    };
    let terminals = services.terminals.clone();
    gpui_kit::application().with_assets(UiAssets).with_quit_mode(QuitMode::LastWindowClosed).run(move |cx| {
        gpui_kit::init(cx);
        if let Err(error) = fonts::register(cx) {
            error!(error = %error, "bundled fonts not registered; system fonts are used");
        }
        if let Err(error) = ThinkrailTheme::install(args.appearance, None, cx) {
            error!(error, "theme not installed");
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some(strings::shell::WINDOW_TITLE.into()), ..Default::default() }),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| workbench::Workbench::new(services, channels, window, cx))
        });
        if let Err(error) = opened {
            error!(error = %format!("{error:#}"), "cannot open window");
            cx.quit();
        }
    });
    terminals.shutdown();
    info!("app exited");
}

/// Reports a failure that happens before logging exists (04 §3 allows stderr only here), then exits.
fn fail(message: &str) -> ! {
    use std::io::Write as _;
    if writeln!(std::io::stderr(), "ThinkRail Native 无法启动：{message}").is_err() {
        std::process::exit(3);
    }
    std::process::exit(2)
}
