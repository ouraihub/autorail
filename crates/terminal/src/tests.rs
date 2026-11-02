use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use thinkrail_core::{
    AttachRequest, ErrorKind, GridSize, Result, Shell, TerminalEvent, TerminalService,
    TerminalSink, WorkspaceId, WorkspaceRoots,
};

use crate::PtyTerminalService;

struct FixedRoot(PathBuf);

impl WorkspaceRoots for FixedRoot {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

struct ChannelSink(async_channel::Sender<TerminalEvent>);

impl TerminalSink for ChannelSink {
    fn publish(&self, event: TerminalEvent) {
        let _ = self.0.send_blocking(event);
    }
}

fn service(
    dir: &std::path::Path,
    capacity: usize,
) -> (PtyTerminalService, async_channel::Receiver<TerminalEvent>) {
    let (tx, rx) = async_channel::bounded(capacity);
    let svc = PtyTerminalService::new(
        dir,
        Arc::new(FixedRoot(dir.to_path_buf())),
        Arc::new(ChannelSink(tx)),
    );
    (svc, rx)
}

#[test]
fn reservations_reload_under_lock_and_reject_corruption() {
    let directory = tempfile::tempdir().unwrap();
    let (first, _a) = service(directory.path(), 8);
    let (second, _b) = service(directory.path(), 8);
    let w = WorkspaceId::from("w");
    first.reserve(&w, "a", "A").unwrap();
    second.reserve(&w, "b", "B").unwrap();
    first.close(&w, "a").unwrap();
    let tabs = second.tabs(&w).unwrap();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].tab_key, "b");
    assert!(tabs[0].runtime.is_none());
    fs::write(directory.path().join("terminals.json"), b"[broken").unwrap();
    let error = first.reserve(&w, "c", "C").unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Corrupt));
    assert_eq!(
        fs::read(directory.path().join("terminals.json")).unwrap(),
        b"[broken"
    );
}

#[test]
fn invalid_identifiers_and_sizes_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (svc, _rx) = service(directory.path(), 8);
    let w = WorkspaceId::from("w");
    assert_eq!(
        svc.reserve(&w, "", "x").unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    assert_eq!(
        svc.reserve(&w, &"k".repeat(501), "x").unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
    let zero = AttachRequest {
        workspace: w,
        tab_key: "t".into(),
        shell: Shell::Cmd,
        size: GridSize { cols: 0, rows: 24 },
    };
    assert_eq!(
        svc.attach(zero).unwrap_err().kind(),
        Some(ErrorKind::Invalid)
    );
}

#[cfg(windows)]
#[test]
#[ignore = "Starts disposable real Windows ConPTY cmd.exe processes"]
fn windows_pty_lifecycle_smoke() {
    let directory = tempfile::tempdir().unwrap();
    let (svc, rx) = service(directory.path(), 8);
    let w = WorkspaceId::from("smoke");
    let request = || AttachRequest {
        workspace: w.clone(),
        tab_key: "cmd".into(),
        shell: Shell::Cmd,
        size: GridSize { cols: 80, rows: 24 },
    };
    let attached = svc.attach(request()).unwrap();
    assert!(attached.created);
    let id = attached.runtime.clone();
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let (left, right) = unique.split_at(16);
    let marker = format!("THINKRAIL_{left}{right}");
    svc.write(
        &id,
        &format!("set \"A=THINKRAIL_{left}\"\rset \"B={right}\"\recho %A%%B%\r"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut output, mut offset) = (String::new(), 0u64);
    while !output.contains(&marker) {
        assert!(
            Instant::now() < deadline,
            "no shell output marker: {output:?}"
        );
        match rx.try_recv() {
            Ok(TerminalEvent::Data {
                runtime,
                byte_offset,
                data,
            }) => {
                assert_eq!(runtime, id);
                assert_eq!(byte_offset, offset);
                offset += data.len() as u64;
                output.push_str(&data);
            }
            Ok(TerminalEvent::Exit { .. }) => panic!("shell exited early"),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    svc.resize(
        &id,
        GridSize {
            cols: 101,
            rows: 37,
        },
    )
    .unwrap();
    let replay = svc.attach(request()).unwrap();
    assert!(!replay.created);
    assert_eq!(replay.runtime, id);
    assert!(replay.replay.contains(&marker));
    assert!(replay.replay_end_offset >= offset);
    assert_eq!(
        replay.size,
        GridSize {
            cols: 101,
            rows: 37
        }
    );
    svc.write(&id, "exit 7\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline, "no natural exit");
        match rx.try_recv() {
            Ok(TerminalEvent::Exit { runtime, exit_code }) => {
                assert_eq!(runtime, id);
                assert_eq!(exit_code, Some(7));
                break;
            }
            Ok(_) => {}
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    thread::sleep(Duration::from_millis(100));
    assert!(svc.tabs(&w).unwrap()[0].runtime.is_none());
    assert_eq!(
        svc.write(&id, "echo dead\r").unwrap_err().kind(),
        Some(ErrorKind::NotFound)
    );
    let fresh = svc.attach(request()).unwrap();
    assert_ne!(fresh.runtime, id);
    svc.close(&w, "cmd").unwrap();
    assert!(svc.tabs(&w).unwrap().is_empty());
    let busy = svc.attach(request()).unwrap();
    svc.write(
        &busy.runtime,
        "for /L %i in (1,1,10000) do @echo BACKPRESSURE_%i\r",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while rx.len() < 8 {
        assert!(Instant::now() < deadline, "bounded queue never filled");
        thread::sleep(Duration::from_millis(5));
    }
    let closing = Instant::now();
    svc.shutdown();
    assert!(
        closing.elapsed() < Duration::from_secs(10),
        "shutdown stalled under backpressure"
    );
    let (restarted, _r) = service(directory.path(), 8);
    assert!(restarted.tabs(&w).unwrap()[0].runtime.is_none());
}
