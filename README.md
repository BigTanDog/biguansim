# 闭关模拟器（biguansim）

Windows 桌面工具：自愿设定学习时长后全屏接管电脑，中途退出需等待冷却并通过答题。全程离线、无账号、无遥测。

单人自用 + 小范围分发（≤10 人）。当前状态：**v0.1 开发中（M1 骨架）**。

## 环境要求

| 项       | 版本                                    |
| -------- | --------------------------------------- |
| Windows  | 10 1809+ / 11，x64                      |
| Rust     | 1.99.1（由 `rust-toolchain.toml` 锁定） |
| MSVC     | VS 2022 Build Tools（VCTools 工作负载） |
| Node     | ≥ 20.19（开发期用 24.x）                |
| pnpm     | 11.x                                    |
| WebView2 | 随 Edge 安装；缺失时 Tauri 会提示       |

## 构建与运行

```bash
pnpm install          # 安装前端依赖
pnpm tauri dev        # 开发模式（Vite + Tauri 窗口）
pnpm tauri build      # 产出 src-tauri/target/release/biguansim.exe
```

> 复验前请**新开一个终端**：`RUSTUP_HOME` / `CARGO_HOME` 写在用户环境变量里，
> 已在运行的 shell 读不到，会误报 `no default toolchain configured`。
> 脚本里一次性调用时显式带上 `RUSTUP_HOME=D:\DevTools\rustup`、`CARGO_HOME=D:\DevTools\cargo`。
> debug 版走 `devUrl`，需要 `pnpm dev` 或 `pnpm tauri dev` 配合；单独双击 debug exe 会显示"localhost 拒绝连接"。

自动化检查（提交前必须全绿，逐条对应 AGENTS §1）：

```bash
pnpm verify           # format:check → lint → typecheck → cargo fmt → cargo clippy
pnpm test             # vitest（仅纯函数）
cargo test --manifest-path src-tauri/Cargo.toml
```

## 数据位置

`%APPDATA%\闭关模拟器\`：`config.json`、`records.jsonl`、`session.json`、`bank\seed.json`、`bank\stats.json`、`logs\biguansim.log`。

## 已知局限（M3 结界阶段，2026-10-20）

1. **破戒通道尚未接入**：键盘钩子与抢焦点已上线（长按 Esc 也在计时），但 8 秒后进二次确认的流程属 M4；闭关中退出请用任务管理器（记「异常中断」）。
2. **亮度不做**：台式机显示器亮度由物理按键控制，DDC/CI 与 WMI 都改不了，已延后到 v1.0 之后（`decisions/005`）。
3. **`Win+Tab` / 虚拟桌面待复验**：早期无钩子版本能切到"桌面 2"，真钩子上线后需要再确认一次。
4. **开发期逃生口**：`pnpm tauri dev` 下闭关页右下角有「DEV 退出（不记录）」按钮（或 `Ctrl+Alt+Shift+X`）；release 构建里这条命令不存在。
5. **多屏未验证**：开发机只有一块 1920×1080 屏幕，副屏镜像与 A-2 无法实测。
6. **钟鸣**：Web Audio 合成，末 10 秒三声滴答、到点三声钟鸣；系统静音时听不到，静音检测在 M5。
7. **结算页**：只显示本次时长与结果，累计/连续天数/评语在 M5。
8. **`Ctrl+Alt+Del` 拦不住**，这是 Windows 的安全设计，不是缺陷（详见 PRD §13）。
9. 本软件不联网、不做完整性校验，本质是"防手滑"而不是"防黑客"。

## 文档

| 文档                          | 作用                                         |
| ----------------------------- | -------------------------------------------- |
| `AGENTS.md`                   | 仓库约束（技术栈、工作流、红线、交付物规范） |
| `docs/闭关模拟器-PRD-v0.1.md` | 产品口径唯一来源                             |
| `docs/闭关模拟器-TDD-v0.1.md` | 技术设计与接口                               |
| `docs/plans/`                 | 总计划与各阶段计划（含执行看板）             |
| `docs/decisions/`             | 决策记录（一条决策一个文件）                 |
