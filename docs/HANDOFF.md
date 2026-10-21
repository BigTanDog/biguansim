# 交接文档（2026-10-21 更新）

> 面向"接着往下写的人"。看完这一份就能开工，不必重读全部历史。
> 详细背景仍以 `AGENTS.md`（约束）、`docs/闭关模拟器-PRD-v0.1.md`（产品）、`docs/闭关模拟器-TDD-v0.1.md`（技术）为准。

---

## 1. 一页速览

**这是什么**：Windows 桌面工具「闭关模拟器」。用户自愿设定学习时长，程序全屏接管电脑；中途退出需冷却 + 答题。离线、单机、无账号。单人自用 + ≤10 人小范围分发。

**现在到哪了**：M0 / M1（骨架）/ M2（流程）/ M3（结界）/ **M4（退出：冷却 + 问阵 + 抄写 + 急事 + `Win+D` 收回）代码全部完成**。M4 的 9 条人工验收（A-4~A-8、A-21~A-23、A-25a，见 `docs/plans/2026-10-20-M4-退出.md` §5）需要你上手。

**下一阶段**：M5「内容与统计」（记录页 / 统计 / 60 题种子 / 记忆衰减与三级退化 / 语料补到 180 条 / 首次引导，估 3 d）。M5 计划尚未写。

```powershell
# 日常三件套（新开终端，环境变量才读得到）
cd d:\CodeBuddy_text\biguansim
pnpm tauri dev            # 开发（Vite + Tauri 窗口，debug 构建依赖 Vite）
pnpm verify               # 门禁：prettier → eslint → vue-tsc → cargo fmt → cargo clippy
cargo test --manifest-path src-tauri/Cargo.toml   # 123 条 Rust 单测；pnpm test 17 条
```

---

## 2. 环境（都已装好，不要再装一遍）

| 项 | 值 |
|---|---|
| 仓库 | `d:\CodeBuddy_text\biguansim`，远端 `git@github.com:BigTanDog/biguansim.git`（**私有**，SSH） |
| Rust | `1.99.1`（named toolchain 已装），`RUSTUP_HOME=D:\DevTools\rustup`、`CARGO_HOME=D:\DevTools\cargo` |
| crates 镜像 | `D:\DevTools\cargo\config.toml` → rsproxy 稀疏索引（官方源本机超时） |
| MSVC | `D:\DevTools\VSBuildTools`（MSVC 14.44 + Win10 SDK 26100） |
| Node / pnpm | `D:\nodejs`（v24.18.0）/ pnpm 11.x |
| WebView2 | 155.0.4283.60（随 Edge） |

**两个必须知道的坑**：

1. `RUSTUP_HOME`/`CARGO_HOME` 是**用户级环境变量**，已在运行的终端/工具进程读不到。报 `no default toolchain configured` 时**换新终端**，不是环境坏了。
2. `cargo` 不在系统 PATH 时，显式加：`$env:Path += ';D:\DevTools\cargo\bin'`。

---

## 3. 仓库规则（违反会被审阅打回）

来自 `AGENTS.md`，这里只列最容易踩的：

- **提交**：`type: 中文描述`（feat / fix / docs / refactor / test / chore），正文写"为什么"。一次提交只做一件事。
- **门禁**：提交前 `pnpm verify` 必须绿。**取退出码要单独取**（`$out = (pnpm run verify 2>&1 | Out-String); $LASTEXITCODE`）——用 `| rg` 取到的是 rg 的 0，会误报绿。
- **红线**：钩子回调 / 主线程 / 抢焦点回调内**禁止** IO、堆分配、加锁、阻塞调用；`Ctrl+Alt+Del` 永不尝试拦截；**出关路径不得停止 tick/io 线程**（P0 教训，已写进 AGENTS §6）。
- **文档同步**：改接口必须回写 TDD；改产品口径必须回写 PRD 并升版本；一条决策一个文件在 `docs/decisions/`。
- **依赖**：新增**直接依赖**需说明理由与体积影响；传递依赖由工具解析（2026-10-20 修订）。

---

## 4. 代码地图与不变量

```
src/                      Vue3 + TS（Pinia，无 router，按 state 切视图）
  App.vue                 视图路由 + toast + 闭关期屏蔽 F5/右键 + DEV 组合键
  views/                  Setup / Arming / Lock（五投影）/ Settled
  components/             ExitPanel（冷却+问阵）/ CopyPanel / ConfirmPanel / EscRing
  stores/session.ts       只存 Rust 推来的投影，不做业务派生
  lib/ipc.ts              唯一允许调 Tauri 的地方（类型化封装）
  lib/chime.ts            Web Audio 合成钟鸣（无音频文件）
  lib/format.ts           倒计时/校验时钟格式化（有 vitest）
  lib/copyText.ts         抄写的展示层比对（有 vitest；判定仍在 Rust）

src-tauri/src/
  lib.rs                  装配：插件、命令注册（dev_abort 只在 debug 编译）、WebView2 参数
  commands/mod.rs         bootstrap / start_session / cancel_arming / get_state (+dev_abort)
  commands/exit.rs        request_unlock / submit_choice / submit_copy / confirm|cancel_emergency
  runtime/mod.rs          tick/io/power/display 线程编排、起势、快照
  runtime/publish.rs      状态投影的唯一构造处（五种活跃态 payload）
  runtime/finish.rs       **出关善后唯一入口**（八步顺序固定）
  io_queue.rs             落盘队列唯一消费者（有"先写后删"回归测试）
  events.rs               IPC 推送载荷（含 PhasePayload）
  app_state.rs            locked 标志（窗口关闭拦截与置顶线程用）
  core/session/{mod,tests}.rs  八态状态机（单一入口 transition）
  core/cost.rs            退出成本公式（纯函数）
  core/bank.rs            题库加载/校验/抽题（内嵌兜底）
  core/challenge.rs       选项打乱与逐题判定（答案不出 Rust）
  core/copy.rs            抄写逐字校验
  core/quote.rs           语料阶段/取句/变量渲染/抄写段
  core/rng.rs             xorshift64*（不引 rand）
  core/timer.rs           单调计时 + 睡眠差分（依赖注入时钟）
  core/store.rs           原子写、JSONL、坏配置备份、坏快照清理
  platform/hook.rs        低级键盘钩子 + 60s 重装 + 每秒键态校正 + Esc 计时/消费
  platform/keyfilter.rs   **纯函数**按键策略（20 条单测，可脱离系统测试）
  platform/focus.rs       抢焦点（事件即抢 + 500ms 兜底 + AttachThreadInput + 最小化还原 + pause）
  platform/sleep.rs       阻止系统睡眠（同线程 SetThreadExecutionState）
  platform/display.rs     显示变化检测（1s 轮询，连续 2 次才判 accident）
  platform/overlay.rs     全屏结界、副屏镜像窗口、置顶维持 + 最小化还原
  platform/log.rs         滚动日志（2×1MB，%APPDATA%\闭关模拟器\logs）
  platform/{power,time}.rs 电源事件订阅、系统时钟实现
```

**五条必须守住的不变量**：

1. **锁顺序 `timer → machine`**，且 timer 的 guard 必须在取 machine 之前释放（反了会 ABBA 死锁，表现为倒计时冻结 + IPC 挂起）。
2. **tick/io 是进程级常驻线程**，出关只投 `IoJob::ClearSnapshot`，绝不停线程。
3. **所有状态转移只走 `core::session::transition`**；到点判定的前置条件是 `active（Locked/Cool/Challenge/Copying/Confirming）&& session.is_some() && elapsed >= target`（少了会误结算）。
4. **前端不派生业务状态**：时间、剩余、该显示什么，都由 Rust 推送；投影只在 `runtime::publish` 构造。
5. **成本与题目都在 Rust**：`cooldown/question_count/copy_chars` 只由 `core::cost` 算；选项打乱与判定只在 `core::challenge`，任何载荷不含 `answer`。

---

## 5. 已完成能力与**待人验证**清单

| 能力 | 状态 |
|---|---|
| 全屏结界（含副屏镜像、5s 可见性兜底） | 已实现；多屏**未验证**（本机单屏） |
| 单调计时（改系统时间不影响、睡眠差分） | 已实现 + 单测 |
| 崩溃恢复（15s 快照、启动不续锁、记 interrupted） | 已实测 |
| 起势 3-2-1 + Esc 取消 + 钟鸣 + 结算页 | 已实测（五连跑 5/5） |
| 键盘钩子（60s 重装、Esc 计时、装不上拒绝入关） | 已实测；A-3 除 `Win` 外全部通过 |
| 抢焦点（≤50ms 抢回） | 已实测 **14 ms**；新增"被最小化（`Win+D`）即还原" |
| 阻止睡眠 | 已实测（`powercfg /requests` 可见本进程 SYSTEM 请求） |
| 显示变化出关 | 去抖逻辑有单测；拔屏**未验证**（单屏机） |
| **退出流程（冷却/问阵/抄写/急事）** | **代码完成 2026-10-21**；A-4~A-8、A-21~A-23、A-25a **待你上手** |
| DEV 逃生口（仅 debug 构建） | 已实测；新增 `Ctrl+Shift+F12` 与醒目按钮 |
| 亮度/压暗 | **延后到 v1.0 之后**（台式机显示器由物理按键控制，无法验证） |

**必须由人手完成的项**（代码侧做不了）：

1. ~~`Win` 键真实键盘复验~~ → **已完成（2026-10-20）：仍唤出，判为已知局限**，见 `decisions/006`。
2. ~~A-3 拦键清单~~ → **已完成**：除 `Win` 外全部通过（见 M3 计划 §7.4）。
3. **`Win+D` 后 1 秒内回到闭关页**（M4 S0 的修复，需真机确认）。
4. **M4 九条**：A-4 / A-5 / A-6 / A-7 / A-8 / A-21 / A-22 / A-23 / A-25a，勾选表在 M4 计划 §5。
5. A-12 拔外接屏 → 2 秒内出关并记 `accident`。
6. A-13 熄屏 15 分钟 → 唤醒仍在闭关、1 秒内正确重绘。
7. 25 分钟满时长完整跑一次（现在可以真的用「解开阵法」出关了）。

---

## 6. 已知问题与技术债

| # | 问题 | 说明与建议 |
|---|---|---|
| 1 | **`Win` 键已定论：拦不住** | 真机复验（2026-10-20）单击仍唤出、`Win+D` 能逃。**不再加码**，PRD §13-9 与 A-3 已按 `decisions/006` 收窄；`Win+D` 的"桌面闪一下"改为 1 秒内收回（S0） |
| 2 | ~~`runtime.rs` 485 行~~ | **已拆**：`runtime/{mod,publish,finish}.rs`，最大 396 行 |
| 3 | ~~`core/session.rs` 374 行~~ | **已拆并扩到八态**：`session/{mod,tests}.rs`，mod 353 行 |
| 7 | 题库只有 12 题、语料每格 3–6 条 | M4 故意只入最小集（D-25）；M5 补到 60 题 / 180 条，并接记忆衰减与三级退化。在那之前「阵法简陋」黄字是真话 |
| 4 | 抢焦点在系统 UI（开始菜单/搜索）持有前台时会失败 | 已用 `AttachThreadInput` 兜底（14ms 成功），但 Start 菜单本身仍可能闪现；根因是它同时是 topmost |
| 5 | 任务栏压制依赖驱动/DPI | 按 PRD §13-8 如实记录，不做假声明 |
| 6 | 静音检测未做 | 系统静音时钟鸣听不到，`config.chimeVolume` 只影响程序侧增益。归 M5 |

---

## 7. 决策索引（都在 `docs/decisions/`）

| 文件 | 一句话 |
|---|---|
| 001 | 总计划一次写全、阶段计划到该阶段启动当天再写 |
| 002 | 私有仓库 + SSH 远端；建仓需人工授权、推送不需要 |
| 003 | 时间戳统一 **Unix 毫秒**（`*Ms` 后缀），不引时间库 |
| 004 | 拦键状态机结论（含 `Win` 三组对照实验） |
| 005 | 结界实测：抢焦点 14ms、`powercfg` 见请求、显示变化单测、亮度延后 |
| 006 | 真机拦键复验：`Win` 拦不住（停止加码 + 收窄 A-3 + PRD §13 待拍板）；`Win+D` 逃逸的 P0 根因与修法 |

---

## 8. 下一阶段 M5 入口

**做什么**（PRD §6.8~§6.11、F-8~F-11）：记录页与统计（累计/连续天数/有效日/破戒）、60 题种子题库、记忆衰减与三级退化（A-15 / A-24）、语料补到 180 条 + 抄写 10 段、设置页、首次引导。验收 A-15 ~ A-18、A-24。

**开工顺序建议**：

1. 先写 `docs/plans/2026-10-2X-M5-内容.md`，把统计口径（有效日跨午夜、断签）与衰减退化规则钉死。
2. `core::bank` 的 `draw` 已经留好扩展点：加 `stats`（`lastAskedMs` / `rightCount` / `wrongCount`）与三级退化即可，**接口不变**。
3. `core::quote` 已有阶段/取句/渲染/抄写抽取；M5 只需补内容与"同局不重复"（`recentQuotes`），并接进闭关页的叙述轮换。
4. 记录页与统计是纯读：`records.jsonl` → `core::stats::summary`，注意本地日期归属发生在统计层（`decisions/003`）。

**M5 之前建议先做**：M4 的九条人工验收 + `Win+D` 收回确认——它们决定 M4 是否可以宣布完成。

---

## 9. 踩过的工程坑（省得重踩）

| 坑 | 现象 | 解法 |
|---|---|---|
| **拿旧 exe 当新版本** | `target/release/biguansim.exe` 是上次 `tauri build` 的产物，改完代码不重跑就还是旧的（2026-10-21 实测踩过：以为 M4 没生效，其实跑的是 M3 的 exe） | 每次改完先看文件时间；启动后查日志有没有 `[bank] 已加载题库 …`（M4 起才有）。`Start-Process` 必须给 `-WorkingDirectory`，否则报"文件名、目录名或卷标语法不正确" |
| **`cargo test` 把可独立运行的 exe 覆盖成连 Vite 的版本** | 裸 `cargo` 构建没有 `custom-protocol` feature，窗口去连 `devUrl`，双击只看到 `localhost 拒绝连接`（2026-10-21 踩过：构建完又跑了一次 `cargo test`，交付的 debug 产物就白了） | 把 `tauri build [--debug]` 放在所有 `cargo test`/`clippy` **之后**；启动日志出现 `[boot] 这是 dev 构建（无 custom-protocol）` 就是坏的 |
| 管道吞掉退出码 | `pnpm verify \| rg …` 拿到的是 rg 的 0，误以为门禁绿 | 用 `Out-String` 后单独取 `$LASTEXITCODE` |
| Vite 监视 `src-tauri` | dev 启动报 `EBUSY … biguansim_lib.dll`，整个 dev 崩掉 | `vite.config.ts` 里 `server.watch.ignored: ['**/src-tauri/**']` |
| debug 版依赖 Vite | 单独双击 debug exe 显示"localhost 拒绝连接" | 用 `pnpm tauri dev`，或跑 release 产物 |
| 窗口"先小窗再全屏" | 全屏后四周漏桌面 | **先 `set_decorations(false)` 再 `set_position/set_size`** |
| 关不掉的窗口 | Idle 用 `decorations:false` 又没自绘标题栏 | Idle 保留原生标题栏，进 Locked 前才去掉 |
| PowerShell 写文件 | `Set-Content -Encoding UTF8` 带 BOM，会把 Rust 源码写坏 | 用 `io.open(..., newline='\n')` 或 `[IO.File]::WriteAllText` + `UTF8Encoding($false)` |
| 中文路径 + `python -c` | 路径变乱码，报 FileNotFoundError | 路径用 `\u` 转义，或改用工具级读写 |
| PowerShell 不支持 heredoc | `python - <<'PY'` 直接语法错 | 分次 `replace_in_file`，或先写临时脚本文件 |
| 编译 example 报 link 1104 | 上一次的 `hook_probe.exe` 还在跑，占着文件 | 先 `Stop-Process -Name hook_probe` |
| Tauri 生成 `gen/schemas` | 每次构建都变，误入库 | 已在 `.gitignore` 忽略 |

---

## 10. 接手第一天检查表

- [ ] 新开终端，跑 `pnpm verify`（单独取退出码）与 `cargo test`，确认 **123 + 17** 全绿。
- [ ] 跑一次 `pnpm tauri dev`，点「入关」，看起势 3-2-1 → 闭关页 → 点「解开阵法」→ 5 分钟冷却 → 答 3 题。
- [ ] 用 DEV 退出（右下角按钮或 `Ctrl+Shift+F12`）结束，确认 `session.json` 已删、记录数不变。
- [ ] 真机确认 `Win+D` 后 1 秒内回到闭关页，且不再出现"桌面 + 键盘锁死"。
- [ ] 读 `docs/decisions/006` 与 M4 计划 §5，按勾选表把 A-4~A-8 / A-21~A-23 / A-25a 过一遍。
- [ ] 再动 M5：先写阶段计划，从记录页与统计口径开始。
