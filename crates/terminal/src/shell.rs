use portable_pty::CommandBuilder;
use thinkrail_core::{Error, Result, Shell};

/// Executable for an explicitly chosen shell; shells foreign to this platform are rejected.
pub(crate) fn program(shell: Shell) -> Result<&'static str> {
    match shell {
        Shell::Cmd if cfg!(windows) => Ok("cmd.exe"),
        Shell::PowerShell if cfg!(windows) => Ok("powershell.exe"),
        Shell::Pwsh => Ok(if cfg!(windows) { "pwsh.exe" } else { "pwsh" }),
        Shell::Bash if !cfg!(windows) => Ok("/bin/bash"),
        other => Err(Error::invalid(format!(
            "当前平台不支持终端 shell {other:?}，请选择其他 shell"
        ))),
    }
}

pub(crate) fn command(shell: Shell, program: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new(program);
    match shell {
        Shell::Cmd => command.arg("/D"),
        Shell::PowerShell | Shell::Pwsh => command.args(["-NoLogo", "-NoProfile"]),
        Shell::Bash => {}
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command
}
