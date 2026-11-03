//! ThinkRail Native desktop app: composition root and GPUI views; logging is `thinkrail-observability`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appearance;
mod panels;
mod services;
mod state;
mod strings;
#[cfg(test)]
mod testing;
mod workbench;

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use gpui_kit::{AppContext as _, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use thinkrail_ui::{UiAssets, fonts};
use tracing::{error, info};

struct Args {
    data_dir: PathBuf,
    db_path: PathBuf,
}

fn derive_paths(
    data_dir: Option<PathBuf>,
    exe_dir: &std::path::Path,
    user_profile: Option<PathBuf>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    match data_dir {
        Some(dir) => Ok((dir.clone(), dir.join("thinkrail.db"))),
        None => {
            let data_dir = user_profile
                .ok_or_else(|| anyhow::anyhow!("找不到用户目录，请用 --data-dir 指定数据目录"))?
                .join(".autorail");
            Ok((data_dir, exe_dir.join("data").join("thinkrail.db")))
        }
    }
}

fn args() -> anyhow::Result<Args> {
    let mut data_dir = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                data_dir = Some(PathBuf::from(
                    args.next().context("--data-dir 需要一个目录")?,
                ))
            }
            _ => bail!(
                "未知参数：{}（可用：--data-dir <目录>）",
                arg.to_string_lossy()
            ),
        }
    }
    let exe_dir = std::env::current_exe()?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("找不到程序目录，请用 --data-dir 指定数据目录"))?;
    let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let (data_dir, db_path) = derive_paths(data_dir, &exe_dir, user_profile)?;
    Ok(Args { data_dir, db_path })
}

fn main() {
    // An engine launches this same executable as its stdio MCP server; no window, no logging.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "mcp-host")
    {
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
    let log = match thinkrail_observability::init(&args.data_dir) {
        Ok(handle) => std::sync::Arc::new(handle),
        Err(error) => fail(&format!("{error:#}")),
    };
    info!(version = env!("CARGO_PKG_VERSION"), "app starting");
    let (services, channels) = match services::build(&args.data_dir, &args.db_path, log.clone()) {
        Ok(built) => built,
        Err(error) => {
            error!(error = %format!("{error:#}"), "startup failed");
            fail(&format!("{error:#}"))
        }
    };
    let terminals = services.terminals.clone();
    let app_prefs = services.app_prefs.clone();
    let data_dir = args.data_dir.clone();
    gpui_kit::application()
        .with_assets(UiAssets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_kit::init(cx);
            if let Err(error) = fonts::register_bundled(cx) {
                error!(error = %error, "bundled fonts not registered; system fonts are used");
            }
            // Before the window opens, so its first frame already has the user's theme (08 A7).
            appearance::AppearanceState::init(app_prefs, &data_dir, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(strings::shell::WINDOW_TITLE.into()),
                    ..Default::default()
                }),
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
#[cfg(test)]
mod tests {
    use super::derive_paths;
    use std::path::PathBuf;

    #[test]
    fn explicit_data_dir_places_database_inside_it() {
        let (data, db) = derive_paths(
            Some(PathBuf::from("C:/data")),
            PathBuf::from("C:/exe").as_path(),
            None,
        )
        .expect("path derivation");
        assert_eq!(data, PathBuf::from("C:/data"));
        assert_eq!(db, PathBuf::from("C:/data/thinkrail.db"));
    }

    #[test]
    fn default_uses_autorail_and_keeps_database_portable() {
        let (data, db) = derive_paths(
            None,
            PathBuf::from("C:/exe").as_path(),
            Some(PathBuf::from("C:/user")),
        )
        .expect("path derivation");
        assert_eq!(data, PathBuf::from("C:/user/.autorail"));
        assert_eq!(db, PathBuf::from("C:/exe/data/thinkrail.db"));
    }
}
