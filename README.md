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

自动化检查（提交前必须全绿，逐条对应 AGENTS §1）：

```bash
pnpm verify           # format:check → lint → typecheck → cargo fmt → cargo clippy
pnpm test             # vitest（仅纯函数）
cargo test --manifest-path src-tauri/Cargo.toml
```

## 数据位置

`%APPDATA%\闭关模拟器\`：`config.json`、`records.jsonl`、`session.json`、`bank\seed.json`、`bank\stats.json`、`logs\biguansim.log`。

## 已知局限（M1 骨架阶段，2026-10-20）

1. **破戒通道尚未启用**：M1 没有键盘钩子，长按 Esc 无效；退出请用任务管理器结束进程，本次会记为「异常中断」（时长仍计入累计）。
2. **拦键尚未接入主程序**：`keyfilter` 的判定逻辑与 15 条单测已就位，探针在 `src-tauri/examples/hook_probe.rs`，真实拦截要到 M3。
3. **多屏未验证**：开发机只有一块 1920×1080 屏幕，多屏覆盖与副屏镜像只做了代码实现，未经真机验证。
4. **到点自动出关**：逻辑有单测覆盖，但"完整跑满 25 分钟"的人工验证尚未做。
5. **`Ctrl+Alt+Del` 拦不住**，这是 Windows 的安全设计，不是缺陷（详见 PRD §13）。
6. 本软件不联网、不做完整性校验，本质是"防手滑"而不是"防黑客"。

## 文档

| 文档                          | 作用                                           |
| ----------------------------- | ---------------------------------------------- |
| `AGENTS.md`                   | 仓库约束（技术栈、工作流、红军线、交付物规范） |
| `docs/闭关模拟器-PRD-v0.1.md` | 产品口径唯一来源                               |
| `docs/闭关模拟器-TDD-v0.1.md` | 技术设计与接口                                 |
| `docs/plans/`                 | 总计划与各阶段计划（含执行看板）               |
| `docs/decisions/`             | 决策记录（一条决策一个文件）                   |
