@echo off
rem Run cargo with the MSVC environment injected (msvc-env.bat).
rem
rem Why this wrapper exists: Git ships a coreutils link.exe that sits earlier on
rem PATH than the MSVC linker. Running cargo directly picks the wrong one and
rem fails with "extra operand ... Try 'link --help'", which gives no hint that
rem the linker itself was the problem.
rem
rem Arguments go to cargo unchanged; pass --manifest-path when the workspace is
rem not in the current directory.
rem Keep this file CRLF and ASCII-only: cmd.exe reads .bat files in the OEM code
rem page and mis-parses LF-only files.
rem No setlocal: msvc-env.bat must keep its PATH changes for the cargo child.
rem
rem Usage: cargo-msvc.bat build --release
rem        cargo-msvc.bat test --manifest-path src-tauri\Cargo.toml

call "%~dp0msvc-env.bat" || exit /b 1
cargo %*
exit /b %ERRORLEVEL%
