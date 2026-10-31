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
if not exist "%DIST%" mkdir "%DIST%"
copy /y "%TARGET%\thinkrail-native.exe" "%DIST%\thinkrail-native.exe" >nul
if errorlevel 1 goto fail

echo [3/3] Creating the portable archive...
if exist "%ARCHIVE%" del /q "%ARCHIVE%"
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$stage = Join-Path $env:TEMP ('thinkrail-native-zip-' + [guid]::NewGuid().ToString('N')); $payload = Join-Path $stage 'thinkrail-native'; New-Item -ItemType Directory -Path $payload -Force | Out-Null; Copy-Item -Path '%DIST%\thinkrail-native.exe' -Destination $payload -Force; Compress-Archive -Path $payload -DestinationPath '%ARCHIVE%' -CompressionLevel Optimal -Force; Remove-Item $stage -Recurse -Force"
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
