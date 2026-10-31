@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion

rem Keep this file CRLF and ASCII-only (cmd.exe mis-parses LF-only batch files).

rem Same variable and fallback as msvc-env.bat. The name has no project in it on
rem purpose: one downloaded toolchain is shared by every repo on the machine.
if not defined PORTABLE_MSVC_ROOT set "PORTABLE_MSVC_ROOT=D:\Applications\msvc-portable\msvc"

rem MSVC 14.44 and SDK 26100 (Windows 11 24H2).
set "MSVC_VERSION=14.44"
set "SDK_VERSION=26100"
set "GIST_URL=https://gist.githubusercontent.com/mmozeiko/7f3162ec2988e81e56d5c4e22cde9977/raw/portable-msvc.py"

set "YES="
set "FORCE="
for %%F in (%*) do (
  if /i "%%~F"=="--yes"   set "YES=1"
  if /i "%%~F"=="-y"      set "YES=1"
  if /i "%%~F"=="--force" set "FORCE=1"
  if /i "%%~F"=="-f"      set "FORCE=1"
  if /i "%%~F"=="--help"  goto :usage
  if /i "%%~F"=="-h"      goto :usage
)

rem PORTABLE_MSVC_ROOT points at the toolchain itself ; the downloader writes into a
rem 'msvc' subfolder of wherever it runs, so it must run in the parent.
for %%P in ("%PORTABLE_MSVC_ROOT%\..") do set "WORKDIR=%%~fP"

echo ============================================
echo   Portable MSVC setup
echo ============================================
echo.
echo Target:  %PORTABLE_MSVC_ROOT%
echo Work in: %WORKDIR%
echo MSVC %MSVC_VERSION%  /  Windows SDK %SDK_VERSION%  /  x64
echo.

rem Local builds always use this portable toolchain (see msvc-env.bat), even when
rem Visual Studio Build Tools are installed, so a system linker is not a reason
rem to skip the install.

if not exist "%PORTABLE_MSVC_ROOT%\setup_x64.bat" goto :needinstall
echo Already installed. Nothing to do.
if defined FORCE goto :reinstall
goto :verify

:reinstall
echo --force given, re-downloading over the existing copy.
echo.

:needinstall
rem uv provides the interpreter ; the downloader is standard-library only, so no
rem virtualenv or dependency install is needed.
where uv >nul 2>nul
if errorlevel 1 goto :nouv
where curl >nul 2>nul
if errorlevel 1 goto :nocurl

echo This downloads about 455 MB of Microsoft build components into:
echo   %WORKDIR%\msvc
echo.
echo The components are Microsoft's and their licence terms govern their use.
echo The downloader is mmozeiko's portable-msvc.py, fetched from:
echo   %GIST_URL%
echo.
if defined YES goto :confirmed
choice /c YN /n /m "Proceed? [Y/N] "
if errorlevel 2 goto :aborted
:confirmed
echo.

if not exist "%WORKDIR%" mkdir "%WORKDIR%" 2>nul
if not exist "%WORKDIR%" goto :nomkdir

echo [1/2] Fetching portable-msvc.py ...
curl -fsSL "%GIST_URL%" -o "%WORKDIR%\portable-msvc.py"
if errorlevel 1 goto :nodownload
if not exist "%WORKDIR%\portable-msvc.py" goto :nodownload
echo       ok
echo.

echo [2/2] Downloading MSVC %MSVC_VERSION% and SDK %SDK_VERSION% ...
pushd "%WORKDIR%"
uv run --python 3.12 python portable-msvc.py --accept-license --msvc-version %MSVC_VERSION% --sdk-version %SDK_VERSION% --target x64 --host x64
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" goto :nomsvc
echo.

:verify
if not exist "%PORTABLE_MSVC_ROOT%\setup_x64.bat" goto :nosetup
echo Verifying the linker ...
call "%PORTABLE_MSVC_ROOT%\setup_x64.bat" >nul 2>nul
call :haslinker
if errorlevel 1 goto :nolinker
echo   ok
echo.
echo ============================================
echo   Done.
echo ============================================
echo.
if /i not "%PORTABLE_MSVC_ROOT%"=="D:\Applications\msvc-portable\msvc" (
  echo This is not the default location, so make it discoverable:
  echo   setx PORTABLE_MSVC_ROOT "%PORTABLE_MSVC_ROOT%"
  echo   ^(then open a new terminal^)
  echo.
)
echo Next: cargo-msvc.bat build
echo.
echo The %WORKDIR%\msvc folder is self-contained ^(setup_x64.bat uses relative
echo paths^), so it can be copied to another machine instead of re-downloading.
goto :end

:nouv
echo ERROR: uv is not on PATH ; it provides the Python used by the downloader.
echo        Install it from https://docs.astral.sh/uv/ and reopen the terminal.
goto :failed

:nocurl
echo ERROR: curl is not on PATH ^(ships with Windows 10 1803 and later^).
goto :failed

:nomkdir
echo ERROR: could not create %WORKDIR%
echo        Check the drive exists and is writable.
goto :failed

:nodownload
echo ERROR: could not download portable-msvc.py
echo        Check network access to gist.githubusercontent.com.
goto :failed

:nomsvc
echo ERROR: the component download failed ^(exit code %RC%^).
echo        Re-running is safe: completed downloads are cached in
echo        %WORKDIR%\downloads
goto :failed

:nosetup
echo ERROR: setup_x64.bat is missing from %PORTABLE_MSVC_ROOT%
echo        The download may have been interrupted. Re-run with --force.
goto :failed

:nolinker
echo ERROR: setup_x64.bat ran but no MSVC linker was found afterwards.
echo        The most likely cause is Git's coreutils link.exe shadowing the MSVC
echo        one on PATH. Run: where link.exe
goto :failed

rem The first link.exe on PATH is the one cargo invokes. Git ships a coreutils
rem 'link' that shadows the MSVC one, and link.exe returns non-zero even for help,
rem so identify the real linker by its banner rather than by an exit code.
:haslinker
for /f "delims=" %%L in ('where link.exe 2^>nul') do (
  "%%L" 2>nul | findstr /i /c:"Microsoft (R) Incremental Linker" >nul && exit /b 0
  exit /b 1
)
exit /b 1

:usage
echo Usage: setup-msvc.bat [--force] [--yes]
echo.
echo   Downloads the portable MSVC toolchain that local builds always use
echo   ^(msvc-env.bat never falls back to Visual Studio Build Tools^).
echo.
echo   --force, -f  re-download over an existing copy
echo   --yes, -y    skip the confirmation prompt
echo.
echo   Set PORTABLE_MSVC_ROOT to install somewhere other than
echo   D:\Applications\msvc-portable\msvc
exit /b 0

:aborted
echo Aborted. Nothing was downloaded.
echo.
if not defined YES pause
exit /b 1

:failed
echo.
if not defined YES pause
exit /b 1

:end
echo.
if not defined YES pause
exit /b 0
