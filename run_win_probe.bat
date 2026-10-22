@echo off
chcp 936 >nul
title Win 键诊断探针
cd /d d:/CodeBuddy_text/biguansim/src-tauri

if not exist "target\debug\examples\win_probe.exe" goto :build

echo ==================================================
echo   Win 键诊断探针
echo ==================================================
echo 提示出现后，请用真实键盘按 Win 键 3 次（每次间隔 2 秒）。
echo 脚本会自动结束，日志在 %TEMP%\biguansim-win-probe.log
echo.
target\debug\examples\win_probe.exe 30
echo.
echo 日志已保存：%TEMP%\biguansim-win-probe.log
pause
exit /b 0

:build
echo 未找到已编译的探针，正在编译（首次约 1 分钟）...
set RUSTUP_HOME=D:/DevTools/rustup
set CARGO_HOME=D:/DevTools/cargo
set PATH=%PATH%;D:/DevTools/cargo/bin
cargo build --example win_probe
if errorlevel 1 goto :failed
echo 编译完成，3 秒后开始。
timeout /t 3 /nobreak >nul
target\debug\examples\win_probe.exe 30
echo.
echo 日志已保存：%TEMP%\biguansim-win-probe.log
pause
exit /b 0

:failed
echo 编译失败，请到 d:/CodeBuddy_text/biguansim 下手动执行：
echo   cargo build --manifest-path src-tauri\Cargo.toml --example win_probe
pause
exit /b 1
