//! Real child-process and stdout callback trace isolation at the default INFO level.

use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;
use thinkrail_core::{BackendKind, EngineRouting};

use super::SessionTrace;
use crate::{
    framing::MAX_FRAME,
    process::Process,
    projection::Projection,
    session::{self, Live},
    testing::Fixture,
};

#[derive(Clone)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("log lock poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn real_process_readers_and_reply_callbacks_keep_sessions_isolated_at_info() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = Logs(bytes.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let fixture = Fixture::new();
    let mut sessions = Vec::new();
    let mut ids = Vec::new();
    for chat in ["trace-chat-a", "trace-chat-b"] {
        let mut request = fixture.request(BackendKind::Pi, None, false, EngineRouting::default());
        request.chat = chat.into();
        let trace = Arc::new(SessionTrace::new(&request));
        ids.push(trace.context.trace_id().to_owned());
        #[cfg(windows)]
        let mut command = {
            let root = std::env::var_os("SystemRoot").unwrap();
            let mut command = Command::new(
                std::path::PathBuf::from(root)
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            );
            command.args(["-NoProfile", "-NonInteractive", "-Command",
                "$line=[Console]::ReadLine(); [Console]::Error.WriteLine('secret-stderr-body'); [Console]::WriteLine($line)"]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "IFS= read -r line; printf '%s\\n' 'secret-stderr-body' >&2; printf '%s\\n' \"$line\""]);
            command
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (process, stdout) = Process::spawn(command, MAX_FRAME, trace).unwrap();
        let (_, sink) = crate::projection::tests::projection();
        let projection = Projection::new(session::identity(&request, Default::default()), sink);
        let live = Live::new(process, projection, BackendKind::Pi);
        let reader = live.clone();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| {
            let _ = reader.resolve("same-request", Ok(frame));
            Ok(())
        })
        .unwrap();
        sessions.push(live);
    }
    assert_ne!(ids[0], ids[1]);
    let workers: Vec<_> =
        sessions
            .iter()
            .cloned()
            .map(|live| {
                std::thread::spawn(move || {
                    live.request_then("same-request".into(),
            &json!({"id": "same-request", "type": "probe", "body": "secret-prompt-body"}),
            Duration::from_secs(20), |_| tracing::info!("trace.callback.executed")).unwrap();
                    live.shutdown();
                })
            })
            .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    // shutdown waits for process exit; stderr_summary waits for the stderr reader to drain.
    for live in &sessions {
        assert_eq!(live.process.stderr_summary(), "secret-stderr-body");
    }
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    for (index, chat) in ["trace-chat-a", "trace-chat-b"].iter().enumerate() {
        for event in [
            "agent process started",
            "agent process exited",
            "agent.stdin.started",
            "agent.stderr.started",
            "agent.stderr.closed",
            "agent.stdout.started",
            "agent.request.replied",
            "trace.callback.executed",
            "agent session disposed",
        ] {
            let matching: Vec<_> = output
                .lines()
                .filter(|line| line.contains(event) && line.contains(chat))
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "missing or duplicate {event} for {chat}: {output}"
            );
            assert!(matching[0].contains(&ids[index]), "{output}");
            assert!(
                !matching[0].contains(&ids[1 - index]),
                "cross-session trace leakage: {output}"
            );
        }
    }
    assert!(!output.contains("secret-prompt-body"), "{output}");
    assert!(!output.contains("secret-stderr-body"), "{output}");
}

#[test]
fn turn_failure_and_cancellation_do_not_become_success_or_unlock_before_settle() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = Logs(bytes.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let fixture = Fixture::new();
    let request = fixture.request(BackendKind::Pi, None, false, EngineRouting::default());
    let trace = Arc::new(SessionTrace::new(&request));
    let id = trace.context.trace_id().to_owned();
    let (mut projection, _) = crate::projection::tests::projection();
    projection.attach_trace(trace);
    projection.set_running(true);
    projection.cancel_turn("user_cancelled");
    assert!(
        projection.is_running(),
        "cancel waits for the real engine settlement"
    );
    projection.set_running(false);
    projection.set_running(true);
    projection.error(thinkrail_core::SessionError {
        code: "EXTERNAL".into(),
        message: "private response".into(),
        trace_id: None,
    });
    assert!(
        projection.is_running(),
        "error must not bypass settlement ordering"
    );
    assert_eq!(
        projection.snapshot().last_error.and_then(|e| e.trace_id),
        Some(id.clone())
    );
    projection.set_running(false);
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let terminals: Vec<_> = output
        .lines()
        .filter(|line| {
            line.contains("operation=\"agent.turn\"")
                && (line.contains("event=\"CANCEL\"")
                    || line.contains("event=\"ERROR\"")
                    || line.contains("event=\"EXIT\""))
        })
        .collect();
    assert_eq!(terminals.len(), 2, "{output}");
    assert!(terminals.iter().all(|line| line.contains(&id)), "{output}");
    assert_eq!(
        terminals
            .iter()
            .filter(|line| line.contains("event=\"CANCEL\""))
            .count(),
        1
    );
    assert_eq!(
        terminals
            .iter()
            .filter(|line| line.contains("event=\"ERROR\""))
            .count(),
        1
    );
    assert!(!output.contains("private response"));
}

#[test]
fn pending_real_process_request_records_timeout_with_configured_limit() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = Logs(bytes.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let fixture = Fixture::new();
    let request = fixture.request(BackendKind::Pi, None, false, EngineRouting::default());
    let trace = Arc::new(SessionTrace::new(&request));
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "while ($null -ne [Console]::ReadLine()) {}",
        ]);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "while IFS= read -r line; do :; done"]);
        command
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (process, _stdout) = Process::spawn(command, MAX_FRAME, trace).unwrap();
    let (_, sink) = crate::projection::tests::projection();
    let projection = Projection::new(session::identity(&request, Default::default()), sink);
    let live = Live::new(process, projection, BackendKind::Pi);
    let error = live
        .request(
            "timeout-request".into(),
            &json!({"type":"get_state"}),
            Duration::from_millis(20),
        )
        .unwrap_err();
    assert_eq!(error.kind(), Some(thinkrail_core::ErrorKind::Timeout));
    live.shutdown();
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let terminals: Vec<_> = output
        .lines()
        .filter(|line| {
            line.contains("operation=\"agent.request\"") && line.contains("event=\"TIMEOUT\"")
        })
        .collect();
    assert_eq!(terminals.len(), 1, "{output}");
    assert!(terminals[0].contains("limit_ms=20"), "{output}");
    assert!(!output.contains("event=\"UNFINISHED\""), "{output}");
}
