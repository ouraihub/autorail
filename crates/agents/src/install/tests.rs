//! Managed installation cleanup and opt-in real download coverage.

use super::*;

#[test]
fn rejected_download_is_removed_and_cleanup_failure_is_not_reported_as_deleted() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("runtime.zip");
    fs::write(&archive, "untrusted archive").unwrap();
    reject_archive(&archive, "runtime");
    assert!(!archive.exists());
    fs::create_dir(&archive).unwrap();
    fs::write(archive.join("keep"), "still present").unwrap();
    let error = reject_archive(&archive, "runtime");
    assert!(archive.join("keep").is_file());
    assert!(error.to_string().contains("无法删除"));
    assert!(!error.to_string().contains("已删除"));
    assert!(remove_artifact(&archive, false).is_err());
}

/// Real download and install of all three engines into a temp dir (network, minutes).
/// `cargo test -p thinkrail-agents install::tests::installs_all_engines -- --ignored`
#[test]
#[ignore = "downloads Node, Bun and three npm packages"]
fn installs_all_engines() {
    let dir = tempfile::TempDir::new().unwrap();
    let engines = ManagedEngines::new(dir.path());
    for backend in BackendKind::ALL {
        let installed = engines
            .install(backend)
            .unwrap_or_else(|e| panic!("{}: {e}", backend.label()));
        assert_eq!(installed.source, EngineSource::Managed);
        assert_eq!(installed.version.as_deref(), Some(package(backend).version));
        assert_eq!(
            engines.status(backend).unwrap(),
            Some(installed.clone()),
            "status finds the install"
        );
        let Launch::External { executable, args } = installed.launch;
        let out = hidden(&executable)
            .args(&args)
            .arg("--version")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        println!(
            "{} --version -> {} {}",
            backend.label(),
            out.status,
            text.trim()
        );
    }
}
