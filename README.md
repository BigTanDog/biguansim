# 闭关模拟器（biguansim）

Windows 桌面工具：自愿设定学习时长后全屏接管电脑，中途退出需等待冷却并通过答题。全程离线、无账号、无遥测。

单人自用 + 小范围分发（≤10 人）。当前状态：**v0.1.0 已打包（2026-10-22）**，进入 14 天自用观察期。

## 下载

| 包                      | 地址                                                                                           |
| ----------------------- | ---------------------------------------------------------------------------------------------- |
| Windows x64（解压即用） | [`release/biguansim-v0.1.0-win64.zip`](release/biguansim-v0.1.0-win64.zip)                     |
| 直链（GitHub）          | `https://github.com/BigTanDog/biguansim/raw/main/release/biguansim-v0.1.0-win64.zip`           |
| exe 校验和              | `4F0FDBB2CFDCEB7F4B83C4FA3412C7809B38AA39DCD0966E4E2BEB7FD209D60A`（包内 `SHA256.txt` 同此值） |
| zip 校验和              | `B5CA21E56885D1805C143AEECCD42AB2D2EDD5533F5AA6958D56326F0435780F`                             |

包内含 `biguansim.exe` + `使用说明.txt` + `SHA256.txt`；**不附带**源码与个人数据。未做代码签名，SmartScreen 会提示"未知发布者"（更多信息 → 仍要运行）。

## 环境要求

| 项       | 版本                                    |
| -------- | --------------------------------------- |
| Windows  | 10 1809+ / 11，x64                      |
| Rust     | 1.99.1（由 `rust-toolchain.toml` 锁定） |
| MSVC     | VS 2022 Build Tools（VCTools 工作负载） |
| Node     | ≥ 20.19（开发期用 24.x）                |
| pnpm     | 11.x                                    |
| WebView2 | 随 Edge 安装；缺失时 Tauri 会提示       |

> 以上是**编译本项目**所需的环境。**只想用发布包的人不需要这些**：解压后双击 `biguansim.exe` 就能跑，唯一额外要求是系统里有 WebView2（Win11 与较新的 Win10 随 Edge 自带；缺失时程序会提示去微软官网装一次）。

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

> **`src-tauri\target\release\biguansim.exe` 不会自动更新**：`target/` 不入库，改完代码必须重跑
> `pnpm tauri build` 才会产出新 exe。判断手上这份是不是新的，看两处——文件时间，以及启动后
> `%APPDATA%\闭关模拟器\logs\biguansim.log` 里有没有 `[bank] 已加载题库 …` 这一行（M4 起才有）。

> **交付前跑 `pnpm artifact`**（= `tauri build` + `tauri build --debug`）：它会按正确顺序把两个产物都重建一遍。
> 只想要测试版时用 `pnpm artifact:test`。**不要**在它之后再跑 `cargo test`/`cargo build`。
>
> **`cargo build` / `cargo test` 会覆盖 `target\debug\biguansim.exe`，把它变成"要连 Vite"的版本**：
> 只有 Tauri CLI（`tauri build` / `tauri build --debug`）会给 `tauri` crate 加 `custom-protocol`
> feature，从而内嵌前端；裸 `cargo` 构建没有这个 feature，窗口会去连 `devUrl` 并显示
> `localhost 拒绝连接`。**顺序上必须把 `tauri build` 放在所有 `cargo test`/`clippy` 之后**，
> 否则最后留下的就是坏产物（2026-10-21 踩过）。这种构建启动时会在日志里打
> `[boot] 这是 dev 构建（无 custom-protocol）…`。

自动化检查（提交前必须全绿，逐条对应 AGENTS §1）：

```bash
pnpm verify           # format:check → lint → typecheck → cargo fmt → cargo clippy
pnpm test             # vitest（仅纯函数）
cargo test --manifest-path src-tauri/Cargo.toml
```

## 数据位置

`%APPDATA%\闭关模拟器\`：`config.json`、`records.jsonl`、`session.json`、`bank\seed.json`、`bank\stats.json`、`logs\biguansim.log`。

## 已知局限（v0.1.0，2026-10-22）

1. **`Win` 键唤出的界面压不住，只能"出现即收回"**：开始菜单/搜索仍可能闪一下（多数时候看不见），程序在 20~40 ms 内注入 `Esc` 并抢回焦点；`Win+D` 之后大约 1 秒内收回。
2. **亮度不做**：台式机显示器亮度由物理按键控制，DDC/CI 与 WMI 都改不了，延后到 v1.0 之后。
3. **`Ctrl+Alt+Del` 拦不住**：这是 Windows 的安全设计，不是缺陷。
4. **静音检测没做**：系统静音时听不到钟鸣与到点滴答，设置页已注明；可靠读取静音需要 Core Audio，暂无验证手段。
5. **题库 30 题**：答对 30 天内不再出现、答错 30 分钟内不出现；全部在冷却时按"最久未出现"补齐。题量本身偏少，长期会重复。
6. **语料 90 条**（每格 10 条）：同一关内不重复，跨关会重复。
7. **抄写依赖中文输入法**：抄写与破戒确认期间暂停抢焦点与前台哨兵，避免候选窗被顶掉、Esc 被打掉。
8. **多屏未验证**：开发机只有一块 1920×1080，副屏镜像与拔屏出关（A-12）未实测。
9. **记录页每次打开解析两遍 `records.jsonl`**：当前量级无感，记录上千条后再合并成一次解析。
10. **数据是明文**：`config.json` / `records.jsonl` / `bank/stats.json` 都可直接改，删掉 `stats.json` 就能重置记忆衰减。不做加密，也不做完整性校验。
11. **不联网、无账号、无遥测**；本质是"防手滑"，不是"防黑客"，也不是家长控制。
12. **开发期逃生口**：debug 构建下闭关页右下角有「DEV 退出（不记录）」按钮（或 `Ctrl+Shift+F12`）；release 构建里这条命令不存在，按钮也不会渲染。

## 测试版与正式版的差别（自测用）

|              | 测试版 `tauri build --debug` → `target/debug/biguansim.exe` | 正式版 `tauri build` → `target/release/biguansim.exe` |
| ------------ | ----------------------------------------------------------- | ----------------------------------------------------- |
| DEV 退出按钮 | 有（不写记录）                                              | 无                                                    |
| 题目选项     | **正确项固定在第一项**（自测不必对答案）                    | 每次打乱                                              |
| 冷却时长     | **×1/10**（5 分钟 → 30 秒，便于短会话跑到第 3 轮抄写）      | 5 / 10 / 15 / 20 分钟                                 |
| 时长下限     | 10 分钟                                                     | 10 分钟                                               |
| 体积 / 性能  | ~13 MB，慢                                                  | ~3.4 MB，快                                           |

两者都自带前端资源，可直接双击运行，**不依赖 Vite**。测试流程（冷却/问阵/抄写/结算）用测试版；验真实强度（打乱、无逃生口）用正式版。

## 文档

| 文档           | 作用                                                                                                          |
| -------------- | ------------------------------------------------------------------------------------------------------------- |
| `AGENTS.md`    | 仓库约束：技术栈、工作流、三条红线、交付物规范                                                                |
| 产品与技术文档 | PRD / TDD / 阶段计划 / 决策记录属于内部材料，**不随仓库分发**；本 README 的「功能」「已知局限」两节是公开口径 |

看完 `AGENTS.md` 与 `README` 就能开始改代码：模块划分见 `src-tauri/src/`（`core/` 纯逻辑、`platform/` 平台绑定、`runtime/` 线程编排、`commands/` IPC），前端在 `src/`（`views/` 页面、`components/` 组件、`stores/` 状态）。

## 测试版与正式版的差别（自测用）

|              | 测试版 `tauri build --debug` → `target/debug/biguansim.exe` | 正式版 `tauri build` → `target/release/biguansim.exe` |
| ------------ | ----------------------------------------------------------- | ----------------------------------------------------- |
| DEV 退出按钮 | 有（不写记录）                                              | 无                                                    |
| 题目选项     | **正确项固定在第一项**（自测不必对答案）                    | 每次打乱                                              |
| 冷却时长     | **×1/10**（5 分钟 → 30 秒，便于短会话跑到第 3 轮抄写）      | 5 / 10 / 15 / 20 分钟                                 |
| 时长下限     | 10 分钟                                                     | 10 分钟                                               |
| 体积 / 性能  | ~13 MB，慢                                                  | ~3.4 MB，快                                           |

两者都自带前端资源，可直接双击运行，**不依赖 Vite**。测试流程（冷却/问阵/抄写/结算）用测试版；验真实强度（打乱、无逃生口）用正式版。

## 文档

| 文档                          | 作用                                         |
| ----------------------------- | -------------------------------------------- |
| `AGENTS.md`                   | 仓库约束（技术栈、工作流、红线、交付物规范） |
| `docs/闭关模拟器-PRD-v0.1.md` | 产品口径唯一来源：功能编号、验收项、已知局限 |
| `docs/闭关模拟器-TDD-v0.1.md` | 技术设计与接口：模块划分、命令与事件契约     |

阶段计划与决策记录（`docs/plans/`、`docs/decisions/`）属于开发过程材料，**不随仓库分发**；其中与实现相关的结论已并入 PRD / TDD 的对应条目。
