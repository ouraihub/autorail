//! 编码搭档 installs (agent-settings 04 §7.10): a pinned Node / Bun downloaded from its official
//! release and checked against a pinned SHA-256, then the pinned engine package installed under
//! `<data>/coding/<engine>`. Downloads and unpacking use the system `curl.exe` and `tar.exe`
//! (Windows 10 and later ship both); nothing runs package install scripts.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use thinkrail_core::{
    BackendKind, EngineInstall, EngineSetup, EngineSource, Error, Launch, Result,
};
use tracing::{info, warn};

/// A runtime archive: official URL, its SHA-256 and the executable inside it.
struct Runtime {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
    exe: &'static str,
}

// ponytail: Windows x64 only, like the rest of the app; add other targets with their archives.
const NODE: Runtime = Runtime {
    name: "node-v24.22.2-win-x64",
    url: "https://nodejs.org/dist/v24.22.2/node-v24.22.2-win-x64.zip",
    sha256: "ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2",
    exe: "node-v24.22.2-win-x64/node.exe",
};
/// Baseline build: runs on CPUs without AVX2.
const BUN: Runtime = Runtime {
    name: "bun-v1.4.4",
    url: "https://github.com/oven-sh/bun/releases/download/bun-v1.4.4/bun-windows-x64-baseline.zip",
    sha256: "0f20ce6674d4f2dbcc8c5d59fdf99f7ba45f8c3b367e0f5781cbfa30a1c78d99",
    exe: "bun-windows-x64-baseline/bun.exe",
};
const NPM_CLI: &str = "node-v24.22.2-win-x64/node_modules/npm/bin/npm-cli.js";

/// The pinned package of one engine and the runtime that runs it.
struct Package {
    name: &'static str,
    version: &'static str,
    runtime: &'static Runtime,
    /// Command name looked up on `PATH` for a system install.
    command: &'static str,
}

fn package(backend: BackendKind) -> Package {
    match backend {
        BackendKind::Pi => Package {
            name: "@earendil-works/pi-coding-agent",
            version: "1.4.0",
            runtime: &NODE,
            command: "pi",
        },
        BackendKind::Omp => Package {
            name: "@oh-my-pi/pi-coding-agent",
            version: "18.14.2",
            runtime: &BUN,
            command: "omp",
        },
        BackendKind::Dsh => Package {
            name: "@deepseek-ai/dsh",
            version: "0.2.1-rc.5",
            runtime: &NODE,
            command: "dsh",
        },
    }
}

/// Installs engines under `<data>/coding` and finds them there or on `PATH`.
#[derive(Debug)]
pub struct ManagedEngines {
    root: PathBuf,
}

impl ManagedEngines {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("coding"),
        }
    }

    fn runtime_exe(&self, runtime: &Runtime) -> PathBuf {
        self.root
            .join("runtimes")
            .join(runtime.name)
            .join(runtime.exe)
    }

    fn engine_dir(&self, backend: BackendKind) -> PathBuf {
        self.root.join(backend.label().to_lowercase())
    }

    /// The managed install, when its runtime and package are both on disk.
    fn managed(&self, backend: BackendKind) -> Result<Option<EngineInstall>> {
        let pkg = package(backend);
        let exe = self.runtime_exe(pkg.runtime);
        let manifest = self
            .engine_dir(backend)
            .join("node_modules")
            .join(pkg.name)
            .join("package.json");
        if !exe.is_file() || !manifest.is_file() {
            return Ok(None);
        }
        let text = fs::read_to_string(&manifest)
            .map_err(|e| Error::external(format!("无法读取 {}：{e}", manifest.display())))?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|_| Error::corrupt(format!("{} 不是有效的 JSON。", manifest.display())))?;
        let bin = match &json["bin"] {
            Value::String(path) => Some(path.as_str()),
            Value::Object(map) => map.get(pkg.command).and_then(Value::as_str),
            _ => None,
        };
        let Some(bin) = bin else {
            return Err(Error::corrupt(format!("{} 没有声明命令入口。", pkg.name)));
        };
        let entry = manifest.parent().unwrap_or(&self.root).join(bin);
        Ok(Some(EngineInstall {
            source: EngineSource::Managed,
            launch: Launch::External {
                executable: exe,
                args: vec![entry.to_string_lossy().into_owned()],
            },
            version: json["version"].as_str().map(str::to_owned),
        }))
    }
}

/// First `PATH` match for `command` (`.exe`, `.cmd`, `.bat`, bare), outside the app's own data.
fn on_path(command: &str, exclude: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute() && !dir.starts_with(exclude))
        .flat_map(|dir| [".exe", ".cmd", ".bat", ""].map(|ext| dir.join(format!("{command}{ext}"))))
        .find(|candidate| candidate.is_file())
}

/// A system install: an `omp.exe`-style binary, or an npm shim next to `node_modules/<pkg>`
/// run with the runtime found on `PATH`.
// ponytail: the system runtime's version is not checked; an old Node fails at the handshake
// with the engine's own error.
fn system(backend: BackendKind, exclude: &Path) -> Option<EngineInstall> {
    let pkg = package(backend);
    let found = on_path(pkg.command, exclude)?;
    let launch = if found
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        Launch::External {
            executable: found.clone(),
            args: Vec::new(),
        }
    } else {
        let manifest = found
            .parent()?
            .join("node_modules")
            .join(pkg.name)
            .join("package.json");
        let json: Value = serde_json::from_str(&fs::read_to_string(&manifest).ok()?).ok()?;
        let bin = match &json["bin"] {
            Value::String(path) => path.clone(),
            Value::Object(map) => map.get(pkg.command)?.as_str()?.to_owned(),
            _ => return None,
        };
        let runtime = if std::ptr::eq(pkg.runtime, &BUN) {
            "bun"
        } else {
            "node"
        };
        let executable = on_path(runtime, exclude)
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))?;
        let entry = manifest.parent()?.join(bin);
        Launch::External {
            executable,
            args: vec![entry.to_string_lossy().into_owned()],
        }
    };
    Some(EngineInstall {
        source: EngineSource::System,
        launch,
        version: None,
    })
}

fn hidden(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command
}

/// Runs `command`; on failure the error carries the last lines of its output.
fn run(mut command: Command, what: &str) -> Result<()> {
    let output = command
        .output()
        .map_err(|e| Error::external(format!("无法启动{what}：{e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = tail[tail.len().saturating_sub(12)..].join("\n");
    Err(Error::external(format!(
        "{what}失败（{}）：\n{tail}",
        output.status
    )))
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        fs::read(path).map_err(|e| Error::external(format!("无法读取 {}：{e}", path.display())))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |e| Error::external(format!("无法写入 {}：{e}", path.display()))
}

/// Missing artifacts are already clean; every other removal failure remains actionable.
fn remove_artifact(path: &Path, directory: bool) -> std::io::Result<()> {
    let removed = if directory {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match removed {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn reject_archive(archive: &Path, runtime: &str) -> Error {
    match remove_artifact(archive, false) {
        Ok(()) => Error::external(format!(
            "{runtime} 的校验和不符，已删除下载的文件。请检查网络或代理后重试。"
        )),
        Err(error) => {
            warn!(error_kind = ?error.kind(), "agent.install.archive_cleanup_failed");
            Error::external(format!(
                "{runtime} 的校验和不符，且无法删除下载的文件；文件未被使用。请检查目录权限后重试。"
            ))
        }
    }
}

impl ManagedEngines {
    /// Downloads and unpacks `runtime` once; rejects a bad checksum and reports cleanup failure.
    fn ensure_runtime(&self, runtime: &Runtime) -> Result<()> {
        if self.runtime_exe(runtime).is_file() {
            return Ok(());
        }
        let dir = self.root.join("runtimes");
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let archive = dir.join(format!("{}.zip", runtime.name));
        let mut curl = hidden(Path::new("curl.exe"));
        curl.args(["-fsSL", "--retry", "2", "--connect-timeout", "30", "-o"])
            .arg(&archive)
            .arg(runtime.url);
        run(curl, &format!("下载 {}", runtime.name))?;
        let actual = sha256_file(&archive)?;
        if actual != runtime.sha256 {
            return Err(reject_archive(&archive, runtime.name));
        }
        // Unpack beside the target and rename, so a half-unpacked runtime is never used.
        let staging = dir.join(format!("{}.part", runtime.name));
        remove_artifact(&staging, true).map_err(io(&staging))?;
        fs::create_dir_all(&staging).map_err(io(&staging))?;
        let mut tar = hidden(Path::new("tar.exe"));
        tar.arg("-xf").arg(&archive).arg("-C").arg(&staging);
        run(tar, &format!("解压 {}", runtime.name))?;
        let target = dir.join(runtime.name);
        remove_artifact(&target, true).map_err(io(&target))?;
        fs::rename(&staging, &target).map_err(io(&target))?;
        if let Err(error) = remove_artifact(&archive, false) {
            warn!(error_kind = ?error.kind(), "agent.install.archive_cleanup_failed");
        }
        info!(runtime = runtime.name, "runtime installed");
        Ok(())
    }
}

impl EngineSetup for ManagedEngines {
    fn status(&self, backend: BackendKind) -> Result<Option<EngineInstall>> {
        Ok(self
            .managed(backend)?
            .or_else(|| system(backend, &self.root)))
    }

    fn install(&self, backend: BackendKind) -> Result<EngineInstall> {
        let pkg = package(backend);
        self.ensure_runtime(pkg.runtime)?;
        let dir = self.engine_dir(backend);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let spec = format!("{}@{}", pkg.name, pkg.version);
        let runtime = self.runtime_exe(pkg.runtime);
        // The runtime's own directory first on PATH, so anything the installer spawns uses it.
        let path = std::env::join_paths(
            runtime.parent().into_iter().map(Path::to_path_buf).chain(
                std::env::var_os("PATH")
                    .iter()
                    .flat_map(std::env::split_paths),
            ),
        )
        .map_err(|_| Error::external("PATH 无法组合。"))?;
        let mut command = hidden(&runtime);
        command.env("PATH", path).current_dir(&dir);
        if std::ptr::eq(pkg.runtime, &BUN) {
            let manifest = dir.join("package.json");
            if !manifest.is_file() {
                fs::write(&manifest, "{\"private\":true}\n").map_err(io(&manifest))?;
            }
            command.args(["add", "--exact", "--ignore-scripts", &spec]);
        } else {
            let npm = self.root.join("runtimes").join(NODE.name).join(NPM_CLI);
            command
                .arg(npm)
                .args(["install", "--prefix"])
                .arg(&dir)
                .args([
                    "--no-fund",
                    "--no-audit",
                    "--ignore-scripts",
                    "--omit=dev",
                    "--save-exact",
                    &spec,
                ]);
        }
        run(command, &format!("安装 {}", backend.label()))?;
        info!(
            backend = backend.label(),
            version = pkg.version,
            "engine installed"
        );
        self.managed(backend)?.ok_or_else(|| {
            Error::external(format!("{} 安装完成但找不到入口文件。", backend.label()))
        })
    }
}

#[cfg(test)]
mod tests;
