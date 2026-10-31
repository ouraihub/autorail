@echo off
rem Inject the MSVC toolchain into the calling cmd process.
rem
rem Local machines always use the portable MSVC under PORTABLE_MSVC_ROOT
rem (default D:\Applications\msvc-portable\msvc), even when Visual Studio
rem Build Tools are installed or the shell is a Developer Command Prompt.
rem GitHub Actions uses its runner compiler only when GITHUB_ACTIONS is set.
rem A stray local CI=true must not switch the local build into CI mode.
rem
rem Called with `call` from cargo-msvc.bat and from project build scripts. No
rem setlocal: the PATH/LIB/INCLUDE changes must stay in the caller.
rem Keep this file CRLF and ASCII-only.
rem
rem Exit code 0 when cl.exe is ready, 1 otherwise.

if /i "%GITHUB_ACTIONS%"=="true" (
  if not defined RUNNER_TOOL_CACHE exit /b 1
  if not defined VCToolsInstallDir exit /b 1
  "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0compiler-paths.ps1" -Root "%VCToolsInstallDir%."
  if errorlevel 1 exit /b 1
  exit /b 0
)

set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"

if not exist "%MSVC_ROOT%\setup_x64.bat" (
  echo ERROR: portable MSVC not found at %MSVC_ROOT%\setup_x64.bat 1>&2
  echo        Run setup-msvc.bat from msvc-kit, or set PORTABLE_MSVC_ROOT to the folder 1>&2
  echo        that contains setup_x64.bat. 1>&2
  exit /b 1
)

rem setup_x64.bat prepends its bin directories to PATH, so its cl.exe and
rem link.exe win over any Build Tools or Git coreutils copies further down.
call "%MSVC_ROOT%\setup_x64.bat" >nul
if errorlevel 1 exit /b 1

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0compiler-paths.ps1" -Root "%MSVC_ROOT%."
exit /b %ERRORLEVEL%
