@echo off
setlocal

rem Build and collect the ThinkRail Native release EXE.
rem Keep this file CRLF and ASCII-only: cmd.exe reads batch files in the OEM code page.

for %%R in ("%~dp0..\..") do set "ROOT=%%~fR\"
set "TARGET=%ROOT%target\release"
set "DIST=%ROOT%dist\thinkrail-native"
set "ARCHIVE=%ROOT%dist\thinkrail-native-%~1-x64.zip"
if "%~1"=="" set "ARCHIVE=%ROOT%dist\thinkrail-native-x64.zip"

echo ============================================
echo   ThinkRail Native - release package
echo ============================================
echo.

echo [1/3] Building the release EXE...
call "%~dp0cargo-msvc.bat" build --workspace --release
if errorlevel 1 goto fail
if not exist "%TARGET%\thinkrail-native.exe" (
  echo ERROR: release EXE was not created. 1>&2
  echo        %TARGET%\thinkrail-native.exe 1>&2
  goto fail
)

echo [2/3] Collecting the release EXE...
if not exist "%ROOT%dist" mkdir "%ROOT%dist"
if errorlevel 1 goto fail
if not exist "%DIST%" mkdir "%DIST%"
if errorlevel 1 goto fail
copy /y "%TARGET%\thinkrail-native.exe" "%DIST%\thinkrail-native.exe" >nul
if errorlevel 1 goto fail

echo [3/3] Creating the portable archive...
if exist "%ARCHIVE%" del /q "%ARCHIVE%"
if exist "%ARCHIVE%" goto fail
set "POWERSHELL="
for /f "delims=" %%P in ('where pwsh.exe 2^>nul') do if not defined POWERSHELL set "POWERSHELL=%%P"
if not defined POWERSHELL set "POWERSHELL=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if not exist "%POWERSHELL%" set "POWERSHELL="
if not defined POWERSHELL (
  echo ERROR: PowerShell was not found. Install pwsh or Windows PowerShell. 1>&2
  goto fail
)
echo Using PowerShell: %POWERSHELL%
set "THINKRAIL_DIST=%DIST%"
set "THINKRAIL_ARCHIVE=%ARCHIVE%"
"%POWERSHELL%" -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference = 'Stop'; $stage = Join-Path ([IO.Path]::GetTempPath()) ('thinkrail-native-zip-' + [guid]::NewGuid().ToString('N')); $payload = Join-Path $stage 'thinkrail-native'; try { New-Item -ItemType Directory -Path $payload -Force | Out-Null; Copy-Item -LiteralPath (Join-Path $env:THINKRAIL_DIST 'thinkrail-native.exe') -Destination $payload -Force; Compress-Archive -LiteralPath $payload -DestinationPath $env:THINKRAIL_ARCHIVE -CompressionLevel Optimal -Force } finally { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }"
if errorlevel 1 goto fail

echo.
echo ============================================
echo   Done
echo ============================================
echo EXE:   %DIST%\thinkrail-native.exe
echo ZIP:   %ARCHIVE%
echo Data:  %%USERPROFILE%%\.autorail or --data-dir ^<directory^>
exit /b 0

:fail
echo.
echo ERROR: release packaging failed. 1>&2
exit /b 1
