# 闭关模拟器 · 技术设计文档（TDD v0.1）

| 项 | 内容 |
|---|---|
| 版本 | v0.1-rev2（M2 同步：四态状态机、起势与钟鸣事件、tick/io 常驻线程与锁序、spike 结论） |
| 日期 | 2026-10-19 |
| 对应需求 | `闭关模拟器-PRD-v0.1.md`（v0.1-rev3） |
| 状态 | 定稿（作为 M1–M5 的实现依据） |
| 不在本文档范围 | 业务代码、题库内容、文案全文 |

> 阅读方式：第 1–3 节定架构（改起来最贵），第 4–6 节定接口（改起来次贵），第 7–13 节是执行细节。
> 与 PRD 的冲突处理：§15 的 R-1~R-5 回写于 **PRD v0.1-rev1**；R-6~R-13 回写于 **PRD v0.1-rev2**；§14 的 T-1~T-6 已于 **2026-10-19 确认采纳**。
> 此后若再发现两边不一致，产品口径以 PRD 为准、实现细节以本文档为准，并在此处登记。

---

## 1. 架构总览

### 1.1 关键决策：可信层放在 Rust，不在前端

讨论稿里曾写"业务逻辑全在 TS"，**本节推翻它**，原因有三：

1. **前端是不可信执行环境**。Vue 代码跑在 WebView 里，开发期可用 DevTools，任何注入/热更新都能改判定逻辑。把"答对/解锁"的判断放在前端，等于把锁的钥匙交给被锁的人。
2. 前端只能发"我选了第 2 项"，**不能发"我答对了"**。判定必须与答案同侧。
3. 计时需要与系统 API（单调时钟、电源通知）同进程低延迟交互，跨 IPC 抖动会积累误差。

所以：**Rust 侧持有唯一权威状态，前端是纯渲染层。**

### 1.2 分层

```
┌──────────────────────────────────────────────┐
│  UI（Vue 3 + TS）                             │
│  只做：渲染 state、收集输入、发意图            │
│  不做：判定、计时、读写文件                     │
└───────────────┬──────────────────────────────┘
                │ Tauri command（请求-响应）
                │ Tauri event（推送）
┌───────────────▼──────────────────────────────┐
│  Core（Rust，纯逻辑，可单测）                   │
│  session 状态机 · timer 计时 · bank 题库判题    │
│  stats 统计 · quote 语料 · store 持久化         │
└───────────────┬──────────────────────────────┘
                │ trait 注入（Clock/Locker/Power/...）
┌───────────────▼──────────────────────────────┐
│  Platform（Rust，Win32 封装）                   │
│  hook · overlay · focus · power · brightness   │
│  display · single_instance                     │
└──────────────────────────────────────────────┘
```

**依赖方向铁律**：Core 不 `use` Platform 的具体实现，只依赖 trait；Platform 不认识 Core 的领域类型，只暴露能力（`lock_input() -> Result`）。这样才能在没有 Windows 的环境里跑完 Core 的全部单测。

### 1.3 进程与线程模型

单进程 + WebView2 子进程（Tauri 默认）。线程清单：

| # | 线程 | 职责 | 约束 |
|---|---|---|---|
| 1 | main | Tauri/tao 事件循环、窗口管理 | 不得阻塞 > 16 ms |
| 2 | hook | 低级键盘钩子 + `GetMessage` 循环 | 回调内**只做 VK 比对**，禁止 IO/分配/加锁，< 100 µs |
| 3 | focus | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` + 消息循环 | 抢回失败 3 次转降级策略 |
| 4 | power | 订阅挂起/恢复通知 + `SetThreadExecutionState` | 启动与清除必须在**同一线程** |
| 5 | tick | 250 ms 节拍，驱动计时、起势倒计时、钟鸣触发点与 UI 推送 | 只读时钟，不做 IO |
| 6 | io | 落盘队列的唯一消费者（原子写、异步） | 15 s 一次 session 快照；出关用入队删除 |

**tick 与 io 是进程级常驻线程（M1 的 P0 结论，务必保留）**：

- 两条线程**只在进程退出时结束**，任何出关路径都**不得**停线程。第一版在 `teardown()` 里停掉 tick/io 且不重建，导致第二次入关倒计时冻结、永不结算、窗口又关不掉——直接违反"绝不锁死"。
- 空闲期由各自的任务判断跳过（tick 看 `machine.state()`，io 阻塞在 `recv`）。
- 出关删快照**不是**直接删文件，而是向 io 队列投递 `IoJob::ClearSnapshot`：队列有序即"先写后删"，不依赖任何轮询等待，慢盘也不会出现"删了又被写回"。
- **锁顺序全项目统一为 `timer → machine`**，且 timer 的 guard 必须在取 machine 之前释放；`build_snapshot`/`get_state`/tick 三处都遵循。反序会 ABBA 死锁（表现为倒计时冻结 + IPC 挂起）。
- `AppState.locked` 只是给窗口事件与置顶维持线程用的**快速镜像**，true 只由 `start_session` 在展开结界**之前**置位（否则 keeper 线程可能先看到 false 直接退出），`unlock_screen` 幂等清零。

> 钩子与焦点为什么不能放主线程：两者都需要消息循环，且回调一旦阻塞主线程会连带卡住 UI 与 Tauri IPC。独立线程是唯一稳妥做法。

---

## 2. 技术栈落地

| 层 | 选型 | 备注 |
|---|---|---|
| 外壳 | Tauri 2.x | 单 crate，不建 workspace |
| 系统调用 | `windows-sys`（纯 Win32 声明，无 COM 封装）；需要 Core Audio（静音检测）/ WMI（内置屏亮度）时局部引入 `windows` 的对应 feature | 只开用到的 feature。纯 Win32 调用用 `windows-sys` 编译更快、产物体积更小 |
| 前端 | Vue 3 + `<script setup>` + Vite | 无 UI 组件库、无 CSS 框架，纯 CSS 变量 |
| 状态 | Pinia | 仅存"最近一次 Rust 推送的投影"，不做业务派生 |
| 前端语言 | TypeScript（strict） | `noUncheckedIndexedAccess` 打开 |
| 包管理 | 前端 pnpm；Rust cargo | 锁文件必须入库 |
| 存储 | JSON 文件 + 原子写 | 不引入 SQLite |
| 日志 | `log` + 简易文件 writer（滚动 2 份、每份 ≤ 1 MB） | 记录钩子安装结果、亮度原值、异常路径 |
| 单实例 | `tauri-plugin-single-instance` | 第二次启动只唤前台 |

**禁止引入**（与会话记录一致的硬约束，防止反复重提）：本地 HTTP 服务、数据库、UI 组件库、CSS 框架、自动更新插件、遥测 SDK、注册表/服务/自启相关 crate。

---

## 3. 模块设计

### 3.1 Rust 侧

| 模块 | 职责 | 公开接口（要点） |
|---|---|---|
| `core::session` | 八态状态机、转移合法性、成本口径、到点规则（副作用由调用方执行） | `transition(event, elapsed) -> Outcome`、`copy_chars()`、`note_fault()`、`reset()` |
| `core::timer` | 单调计时 + 睡眠累计 + 时间篡改检测 | `elapsed_ms() -> u64`、`suspect_clock_change() -> Option<i64>` |
| `core::cost` | 退出成本公式（纯函数，基数为入参） | `cooldown_min/ms(n, base)`、`question_count(n, base)`、`copy_chars(n)` |
| `core::bank` | 题库加载、校验、抽题（含记忆衰减的退化） | `load(dir)`、`draw(n, &mut Rng)`（不看统计）、`draw_with(&Table, n, now_ms, &mut Rng)`（正式抽题） |
| `core::challenge` | 选项打乱与逐题判定（答案不出 Rust） | `Round::prepare(questions, attempt, rng)`、`submit(choice) -> Judgement`、`view() -> QuestionView` |
| `core::copy` | 抄写任务与逐字校验（不归一化） | `CopyTask::new(text)`、`check(target, typed) -> CopyVerdict` |
| `core::rng` | 可复现随机数（xorshift64*，不引 `rand`） | `Rng::new(seed)`、`below(n)`、`shuffle(&mut [T])` |
| `core::bank_stats` | `bank/stats.json`：冷却窗口与答对/答错计数 | `Table::{load, save, mark, is_cooling, last_asked_ms}` |
| `core::stats` | 累计/连续天数/破戒/有效日（纯函数，日期由调用方换算） | `summary(&[DatedRecord], today: LocalDate) -> Summary`、`LocalDate::yesterday()`、`date_from_unix_ms_utc(ms)` |
| `core::quote` | 语料加载、阶段选择、变量渲染、防重复 | `pick(stage, kind, vars) -> String` |
| `core::store` | 配置/流水/快照的读写与迁移 | `atomic_write(path, bytes)`、`append_jsonl()` |
| `io_queue` | 落盘队列的唯一消费者（进程级常驻） | `run(store, rx)`、`IoJob::{Snapshot, ClearSnapshot, BankStats}` |
| `platform::keyfilter` | 按键判定**纯函数**（不含任何系统调用，可单测） | `decide(vk, is_down, &mut ModState) -> Swallow/Pass` |
| `platform::hook` | 键盘钩子安装/卸载、60 s 周期重装（**先装新的再卸旧的**）、每秒 `GetAsyncKeyState` 键态校正、Esc 长按时间戳 | `install(&AppHandle) -> Result<()>`、`esc_hold_ms() -> u64` |
| `platform::focus` | 抢焦点：前台事件立即抢 + 500 ms 兜底重试 + `AttachThreadInput` 强制；**回调内不做 IO** | `start(hwnd) / stop()` |
| `platform::sleep` | 阻止系统睡眠（同线程 `SetThreadExecutionState`，只加 `ES_SYSTEM_REQUIRED`） | `block() / allow()` |
| `platform::display` | 显示变化检测（1 s 轮询，连续 2 次不一致才判 `accident`） | `spawn(app, runtime)` |
| `platform::overlay` | 多屏窗口创建、置顶、销毁 | `show_all() -> Vec<WindowId>`、`hide_all()` |
| `platform::focus` | 前台窗口监听与抢回 | `arm(targets) / disarm()` |
| `platform::power` | 阻止睡眠、订阅挂起/恢复 | `block_sleep() -> Guard`、`watch(cb)` |
| `platform::brightness` | 亮度基线记录、降亮、还原 | `capture() -> Baseline`、`dim(50%)`、`restore(Baseline)` |
| `platform::display` | 显示器快照轮询、变化判定 | `snapshot() -> DisplaySnapshot`、`watch(cb)` |
| `commands` | IPC 命令（唯一对外入口） | 见 §5.1 |
| `events` | IPC 推送 | 见 §5.2 |

### 3.2 前端侧

| 模块 | 职责 |
|---|---|
| `views/Setup.vue` | 配置页（时长/阵法/目标/入关） |
| `views/Lock.vue` | 闭关页（环+倒计时+对话框，承载 Locked/Cool/Challenge/Copying/Confirming 五种投影） |
| `views/Settle.vue` | 结算页 |
| `views/Records.vue` | 记录页 |
| `views/Bank.vue` | 题库页（只读） |
| `views/Settings.vue` | 设置页 |
| `views/Onboarding.vue` | 首次引导三屏 |
| `components/DialogBox.vue` | 底部对话框（叙述/选项/输入三态） |
| `components/Ring.vue` | 细环进度（仅弧长，无数字） |
| `stores/session.ts` | 保存最近一次 state 投影 |
| `lib/ipc.ts` | 全部 `invoke` 的类型化封装（**唯一**可直接调用 Tauri 的文件） |

> `Lock.vue` 不按状态拆多个页面，而是**一个页面五种投影**——避免冷却/答题/抄写切换时重建窗口导致的闪烁与焦点丢失（这在全屏锁场景下会造成"窗口闪一下露出桌面"的严重观感问题）。

---

## 4. 关键机制设计

### 4.1 计时（最重要的技术点）

**问题**：PRD 初版（v0.1）F-3.15/F-3.16 写的是"取单调值与墙钟的较大值"——用户把系统时间调快 1 小时，墙钟增量立刻变大，取较大值等于**送他 1 小时**，与验收用例 A-9 直接冲突。**该条已在 PRD v0.1-rev1 修订为本节算法。**

**方案：单调时钟为主 + 睡眠事件单独累加。**

```
lock 时刻：
  mono0   = QueryUnbiasedInterruptTime()     // 不含传统睡眠，不受调时影响
  wall0   = GetSystemTimePreciseAsFileTime()
  sleepAccum = 0

每次 tick：
  mono  = (QueryUnbiasedInterruptTime() - mono0) / 10_000      // ms
  wall  = (now_wall - wall0) / 10_000
  elapsed = mono + sleepAccum
```

睡眠如何处理：
- `PowerRegisterSuspendResumeNotification`（`DEVICE_NOTIFY_CALLBACK`）收到 `PBT_APMSUSPEND` → 同时记下 `suspendMono` 与 `suspendWall`；
- 收到 `PBT_APMRESUMEAUTOMATIC` **或** `PBT_APMRESUMESUSPEND`（两条都必须处理：系统设置了"唤醒需要密码"时只来后者）→ 按下式累加，并立即重算。

**必须用差分，不能直接用墙钟差**：

```
wallDelta = now_wall - suspendWall
monoDelta = now_mono - suspendMono
sleepAccum += max(0, wallDelta - monoDelta)
```

| 机型 / 场景 | `monoDelta` | 结果 |
|---|---|---|
| 传统 S3 睡眠 | ≈ 0 | `sleepAccum += wallDelta`，符合 PRD A-10 |
| **Modern Standby（S0ix）** | 可能 ≈ `wallDelta`（S0 低功耗待机期间内核中断时间仍在走） | 差额 ≈ 0，**不重复累加**——避免"睡 10 分钟扣 20 分钟" |
| 混合待机（先 S0ix 后转 S3） | 介于两者之间 | 只补中断时间没走的那部分 |

> 这张表必须在**你的目标机型**上实测确认（交付时标注"需要你确认"）：先 `powercfg /a` 看是否支持 S0ix，再手动待机 10 分钟核对 `sleepAccum` 增量。若实测与表格不符，先修公式再谈 M1 验收，不要用"应该没问题"跳过。

篡改检测（只用于提示，不影响结算）：

| 条件 | 判定 | 处理 |
|---|---|---|
| `wall - elapsed` 绝对值 ≤ 3 分钟 | 正常 | 校验时钟显示「一致」 |
| `wall - elapsed` > 3 分钟且无睡眠事件 | 时间被调快 | 显示「不一致 (+N 分)」，**结算仍用 elapsed**（用户不得利） |
| `wall - elapsed` < -3 分钟 | 时间被回拨 | 显示「不一致 (-N 分)」，结算仍用 elapsed |

**关键性质**：
- 调快 / 调慢系统时间 → `elapsed` 不变 → 剩余时间不变 ✅（PRD A-9）
- 传统睡眠 10 分钟 → `sleepAccum` 补 10 分钟 ✅（PRD A-10）
- Modern Standby 待机 10 分钟 → 差分 ≈ 0，不重复累加 ✅（A-10 的新增子用例）
- **休眠（S4）不会杀死进程**：内存镜像恢复后进程继续运行，走 resume 通知路径。只有关机 / 重启 / 断电才走 `Recovered` 崩溃恢复路径、记 `interrupted`。（初版"S4 = 进程已死"的说法是错的，会误导测试设计）

### 4.2 显式睡眠事件的必要性

若不装电源通知，只能靠墙钟猜，会把"调时"和"睡眠"混为一谈，二者必有一伤。装通知后两者可区分，这是本设计对 PRD 的一处实质修正（§15 R-1）。

### 4.3 全屏窗口策略

| 决策 | 选择 | 理由 |
|---|---|---|
| 是否用真·独占全屏 | **否**，用无边框 + 精确位图尺寸 + `always_on_top` | 独占全屏在 DWM 下切换有黑屏动画，且多屏难以处理 |
| 主屏窗口 | 复用主窗口，切换到 Lock 视图并重定位 | 避免新旧窗口交替时的"露出桌面"瞬间 |
| 窗口几何 | 尺寸取 `MonitorInfo.rcMonitor`（**含任务栏区域**），不用 `rcWork`；位置按物理像素经 `SetWindowPos` 设置 | 任务栏右键 →「任务管理器」是鼠标路径，键盘钩子拦不到；覆盖整屏是不调 AppBar API 的唯一堵法（PRD §13-8） |
| Z 序维持 | 抢焦点时顺带 `SetWindowPos(HWND_TOPMOST)`；进入 Locked 后每 1 s 重申一次 | 任务栏本身也是 topmost，只在创建时置顶会被它压回去 |
| 副屏窗口 | 额外创建，加载同一前端，路由带 `?display=2`，只显示环 + 倒计时镜像；设 `focusable: false` + `WS_EX_NOACTIVATE`，且不进入抢焦点目标集合 | 交互（对话框）只在主屏；副屏可激活会让焦点在两块屏之间来回抖 |
| 任务栏 | 不隐藏（不调 AppBar API），靠置顶覆盖 | 修改任务栏状态属于"系统侵入"，出问题难还原 |
| 光标 | 不隐藏、不裁剪 | 无实际收益，且会引发用户恐慌 |
| 关闭事件 | 拦截 `CloseRequested` 与 `WM_CLOSE`，Locked 期间一律拒绝 | 防 Alt+F4 |

### 4.4 键盘钩子

- 类型：`WH_KEYBOARD_LL`，装在专用线程，装完跑 `GetMessage` 循环。
- 回调逻辑：查状态机 → `return 1`（吞掉）。
- 拦截表与放行表见 PRD F-3.3 / F-3.4；两张表用静态 `match`，**不得**在回调里分配内存或读配置文件（每次按键都读配置会累积延迟）。
- **本地日换算只在平台层**：`platform::time::local_date(wall_ms)` 用 `FileTimeToSystemTime` + `SystemTimeToTzSpecificLocalTime`，夏令时与改时区都跟系统走；失败时退回 UTC 日期（只可能让跨午夜的那条归错一天）。`core::stats` 只收 `LocalDate`，不碰系统时间。
- 回调超过 `LowLevelHooksTimeout`（默认 1 s）会被系统静默摘除 → 因此回调内零耗时。
- **`LockWorkStation()` 绝不能写在回调里**：锁屏要切安全桌面，耗时可达数百毫秒，回调超时即被摘钩，之后所有按键放行。回调只 `AtomicBool::store`，由 50 ms 轮询的专用线程执行（M4 计划 D-35）。同理，`app.emit` 也不能在钩子线程的消息循环里做。
- **Esc 长按必须只认"按下沿"**：长按时系统每 ~30 ms 重发一次 `WM_KEYDOWN`，若每次都用"现在"覆盖起点，`esc_hold_ms()` 永远接近 0，8 秒破戒永远触发不了（2026-10-21 真机踩坑）。计时状态打包进一个 `AtomicU64`（held / armed / downMs），判定全部在 `keyfilter::esc_on_key/esc_hold_ms/esc_consume` 这三个**纯函数**里，回调只做 load→store，不加锁、不分配。

**按键状态机（必须实现，不能只做"查表 return 1"）**

Windows 的组合键不是独立按键，只按 VK 查表会出现两种必然错误：吞掉 `Win` 会连带废掉要放行的 `Win+L`；不吞 `Win` 则 `Win+E/D/R/I/S/X` 全部照常生效。

| 状态 | 规则 |
|---|---|
| `Win` 按下 | 记录 `winDown`；为保住 `Win+L`，`Win` 的**按下**必须放行（吞掉它系统就认为 Win 从未按下，`Win+L` 也就不成立）；`winDown` 期间吞掉所有字母键与功能键，`L` 除外（放行 `Win+L`，F-3.4）。**抬起键怎么处理（放行 / 吞掉 / 合成事件）是 spike 的核心问题**：放行会让单按 Win 弹出开始菜单，吞掉可能留下"Win 一直被按住"的系统状态，必须实测取舍 |
| `Alt` 按下 | 记录 `altDown`；期间吞 `Tab`、`Esc`、`F4`，其余放行（避免误伤 AltGr 输入特殊字符） |
| `Ctrl` 按下 | 吞 `Esc`、`Shift+Esc`、`Alt+Tab`（持久切换器） |
| 无修饰键 | 除 `Win` 派生路径外全部放行 |
| 左右 `Win` | `VK_LWIN` / `VK_RWIN` 分别计数，防止左右交替按导致状态错乱 |
| 进入 `Locked` | 清空 `winDown/altDown/ctrlDown`，并把 `Arming` 期间按住的 Esc 计时清零（否则会跨状态误触发长按破戒） |

`Ctrl+Alt+Tab`（**持久**应用切换器，鼠标可点）与 `Win+Ctrl+←/→`（虚拟桌面）必须一并吞掉；漏掉这两条，用户不用任务管理器就能切走。

**Esc 长按判定放在这一层**：钩子线程记录 Esc 的按下/抬起时间戳，`esc_hold_ms()` 满 8000 才允许进入 `Confirming`。前端只渲染进度环——判定不能放在不可信侧（§1.1）。

**钩子探活：用周期重装，不要用消息探针**

"投递空消息检查线程是否还活着"**检测不到**钩子被摘除：系统静默摘钩时线程消息循环完好无损，`PostThreadMessage` 照常成功。正确做法：

- 每 60 s 由 hook 线程自己 `UnhookWindowsHookEx` + `SetWindowsHookEx` 重装一次并校验返回值；失败重试 3 次后上报 `notice://toast`「结界已重新加固」；
- 重装时机选在最近 100 ms 无按键的时刻（`GetLastInputInfo` 判断），避免吞掉一次真实按键；
- 线程存活检测保留作为第二层（`GetMessage` 返回 `-1` 视为异常）。

- 安装失败 → `commands.start_session` 返回 `E_HOOK_FAILED`，**拒绝入关**（PRD D-7）。

> **M3 的第一件事是 spike（半天）**：只验证五件事——`Win+L` 可放行、`Win+E` 被拦、`Alt+Tab` 被拦、**单按 Win 不弹开始菜单**、正常中英文打字与输入法切换不受影响。五项同时成立才继续；不成立就退回"只拦 `Alt+Tab` + `Ctrl+Esc` + `Ctrl+Shift+Esc` + `Alt+F4`"的最小集合，并把 `Win` 组合键列入 §11 的残余风险，不要硬扛。

**spike 实测结论（2026-10-20，前移到 M1 末尾执行）**：

- `Win+L`、`Win+E`、`Alt+Tab` 符合预期，正常打字与输入法不受影响；
- **单击 `Win` 会弹出开始菜单并露出任务栏**（原方案放行 `Win` 的抬起）。修法：`Win` 按下仍放行（否则 `Win+L` 不成立），**抬起时分情况**——整段按住期间没按过别的键就吞掉抬起，按过别的键则放行；
- 低级钩子会上报**左右专用修饰键 VK**（`VK_LCONTROL 0xA2` 等），状态机必须同时认通用与专用两种，否则 `Alt+Tab`/`Ctrl+Shift+Esc` 判定失效、探针的安全退出组合键也失效。
- 参见 `docs/decisions/004-拦键状态机结论.md`。

### 4.5 抢焦点

- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)`，回调在**安装该钩子的线程**（线程 3）的消息队列里投递，因此线程 3 必须有消息循环且不得阻塞——回调被阻塞等于抢焦点失效。
- 判定：当前前台 hwnd 不在我们的窗口集合中 → `SetForegroundWindow(our_hwnd)`。
- 失败处理：连续 3 次失败后记录 warning 并降频到 500 ms 重试（不放弃）。
- 必须排除：本进程的其他窗口、系统安全桌面（收不到事件，无需处理）。

### 4.6 电源与亮度

- 阻止睡眠：在 power 线程 `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`；清除时**同线程**调用 `ES_CONTINUOUS`。跨线程清除在部分机型上不生效。
- 亮度基线：入关前读所有显示器亮度，写 `display_baseline.json`（**磁盘持久化，不只存内存**）——这样即使进程崩溃，下次启动也能检测到残留并**自动还原**（崩溃后屏幕一直发暗会被误认为显示器坏了，见 §4.8）。
- 降亮：内置屏走 WMI `WmiMonitorBrightnessMethods.WmiSetBrightness`；外接屏走 DDC/CI（`GetPhysicalMonitorsFromHMONITOR` + `SetVCPFeature(0x10)`）。任一失败静默跳过，只在日志记一条。
- **两者都是同步阻塞调用，单次可达数百毫秒**：只能在 io 线程执行，入关时"异步发起、不等待结果"，绝不能在主线程 / hook 线程 / 抢焦点回调里调用。WMI 线程需 `CoInitializeEx(COINIT_MULTITHREADED)`，Tauri 主线程的 apartment 不适用。
- 还原：出关时用基线精确回写；失败则保留 `display_baseline.json` 供手动恢复。

### 4.7 显示变化判定

不使用 `WM_DISPLAYCHANGE`（无窗口线程收不到，且熄屏唤醒会误报），改用**快照轮询**：

```
snapshot = { count, [(width, height, scale_factor) ...] }  // 按坐标排序后比对

每 1 s 取一次快照，与基线比对
连续 2 次不一致（≈2 s）→ 判定成立 → 触发 accident 出关
任一次回到基线 → 计数清零
```

去抖靠"连续 2 次"，不靠延迟定时器——避免定时器被漏触发。

### 4.8 崩溃恢复

- Locked 期间每 15 s 原子写 `session.json`（含 `state/targetMs/elapsedMs/monoBase/wallBaseMs/startedAtMs/attempts/cooldownUntilMs/bank/goal`）。
- 正常出关：删除 `session.json`。
- 启动时若存在 `session.json` 且 `state == Locked`：
  1. 不进入锁定，直接进 `Recovered` 弹窗；
  2. 追加一条 `result=interrupted`、`actualMs = 快照中的 elapsedMs` 的记录（**不要写 0**：PRD F-9.4 规定 `interrupted` 计入累计，只是不计有效日；粒度 15 s，最多少记 15 秒）；
  3. 删除 `session.json`；
  4. 若 `display_baseline.json` 存在 → **自动还原亮度**（崩溃后屏幕还暗着会让人以为显示器坏了），还原后弹窗写"上次的亮度已恢复，是否保留该记录"，而不是只做个提示。

### 4.9 单实例

- `tauri-plugin-single-instance`：第二实例启动时向第一实例发事件并退出；第一实例把窗口唤到前台。
- 若第一实例处于 Locked，第二实例**不做任何事**（不能因为它而解锁），只忽略。

---

## 5. IPC 契约

### 5.1 命令（TS → Rust，请求-响应）

| 命令 | 入参 | 返回 | 说明 |
|---|---|---|---|
| `bootstrap` | — | `{ config, stats, todayValid, lastSession }` | 启动时一次 |
| `start_session` | `{ durationMin, goal?, bankId }` | `{ ok, sessionId }` \| `{ error }` | **只进 `Arming`**，3 秒后由 tick 线程转 `Locked`；失败码见 §5.3。成功时写回 `config.lastDurationMin` |
| `cancel_arming` | — | `{}` \| `{ error }` | `Arming` → `Idle`（F-2.2）；非 `Arming` 返回 `E_STATE_ILLEGAL` |
| `request_unlock` | — | `{ accepted, attempt, cooldownMs }` | Locked → Cool；剩余不足一次冷却返回 `E_REMAIN_TOO_SHORT`，状态不变（F-4.5） |
| `submit_choice` | `{ questionId, choice }` | `{ accepted, correct }` | **判定在 Rust**。`choice` 是**打乱后的展示索引（0–3）**；`questionId` 与当前题不符或越界返回 `E_INVALID_CHOICE`。下一题由 `state://changed` 的 `payload` 推送，没有单独的"取题"命令 |
| `submit_copy` | `{ text }` | `{ ok, mismatchIndex? }` | 逐字校验（不归一化全角半角）；返回首个不匹配位置供前端标红 |
| `confirm_emergency` | `{ text }` | `{}` \| `{ error }` | 逐字校验 9 字；不一致返回 `E_CONFIRM_MISMATCH` |
| `cancel_emergency` | — | `{}` | 回 Locked，不消耗成本，Esc 进度清零 |
| `get_state` | — | `SessionState` 全量 | 前端重连/重绘用 |
| `list_records` | `{ limit }` | `Record[]` | 记录页 |
| `list_questions` | — | `QuestionSummary[]` | 题库页（不含答案） |
| `update_config` | `{ patch }` | `{ config }` | 设置页。**浅合并 + 校验 + 原子写**：`cooldownBase ∈ [3,10]`、`questionBase ∈ [3,5]`、`chimeVolume ∈ [0,0.6]`、`theme ∈ {dark, light}`；非法值返回 `E_INVALID_CONFIG` 且**不落盘**（M5） |
| `open_data_dir` | — | `{ ok }` | 打开 `%APPDATA%` |
| `restore_brightness` | — | `{ ok }` | 手动兜底 |

**契约约束**：
- 全部命令在非预期状态下调用，一律返回 `E_STATE_ILLEGAL`，**不得静默成功**。
- 任何命令都不得返回答案字段（`answer` 只在 Rust 内部使用）。
- 选项打乱的映射只存在于 Rust（`questionId → [展示索引 → 原始索引]`），下发的是打乱后的 `options` 数组，前端不持有原始顺序。
- `submit_choice` 与 `submit_copy` 的判定结果只信 Rust 返回，前端不得自行比较或缓存题目内容用于后续判定。

### 5.2 事件（Rust → TS）

| 事件 | 载荷 | 频率 |
|---|---|---|
| `state://changed` | `{ state, result?, goal, elapsedMs, targetMs, payload? }` | 状态转移时（八态全覆盖）。`payload` 按 `kind` 分支：`cool` / `challenge` / `copy` / `confirm`，是**活跃态的唯一投影来源** |
| `timer://tick` | `{ elapsedMs, remainingMs, clockSkewMin }` | 250 ms（五个活跃态都推，倒计时不因进入冷却而中断） |
| `arming://tick` | `{ remainingMs }` | 250 ms（仅 `Arming`，由 Rust 计时，前端只渲染 3-2-1） |
| `cool://tick` | `{ remainMs }` | 250 ms（仅 `Cool`；界面只渲染分钟粒度） |
| `esc://hold` | `{ heldMs }` | 250 ms（仅 `Locked` 且按住 Esc 时；只画进度环，**8 秒判定在 Rust**） |
| `chime://play` | `{ kind: "tick" \| "bell", index }` | 剩余 10/7/4 秒各一次 `tick`；到点一次 `bell`（前端连响 3 声） |
| `notice://toast` | `{ level, text }` | 按需（如"钩子已重装"） |

前端只渲染，不派生：例如"是否该显示冷却语"由 Rust 推送的 `state` 决定，前端不写 `if (elapsed > x)`。

### 5.3 错误码

| 码 | 含义 | UI 表现 |
|---|---|---|
| `E_HOOK_FAILED` | 钩子安装失败 | 「结界未能生效，已取消入关」+ 排查建议 |
| `E_FOCUS_FAILED` | 抢焦点钩子安装失败（M3 起只记日志、不拒绝入关，因为拦键仍在） | 界面提示「未能接管前台」 |
| `E_OVERLAY_FAILED` | 主屏窗口创建失败 | 同上 |
| `E_BANK_INVALID` | 题库解析失败且无内嵌回退 | 禁止入关 |
| `E_STATE_ILLEGAL` | 非法状态调用（含长按不足 8 s 就请求紧急通道） | 前端静默刷新 `get_state` |
| `E_REMAIN_TOO_SHORT` | 剩余时间不足一次冷却（F-4.5） | 对话框显示"距出关不足一次冷却，请稍候到点。"并保持 `Locked` |
| `E_CONFIRM_MISMATCH` | 破戒抄写句不一致（F-6.2） | 输入框标红，不消耗成本 |
| `E_INVALID_CHOICE` | `choice` 越界或题号已失效 | 前端丢弃本次点击、以 `state://changed` 的 `payload` 为准 |
| `E_BRIGHTNESS_FAILED` | 亮度调整/还原失败 | 仅在设置页提示，不打断 |
| `E_STORE_FAILED` | 落盘失败 | 顶部横幅「记录未能保存」 |

### 5.4 前端不可信面的关闭清单（release 前逐项检查）

§1.1 的前提是"前端改不动判定"。这个前提只在下面六项全部成立时才成立，任何一项失效，DevTools 里一句 `invoke('confirm_emergency', {...})` 就能开门。

| 项 | 要求 | 检查方式 |
|---|---|---|
| WebView devtools | 不启用 `devtools` feature；`tauri.conf.json` 不设 `devtools: true`；WebView2 的 `AreDevToolsEnabled = false` | release 构建后按 F12 / Ctrl+Shift+I 无反应 |
| `withGlobalTauri` | `false` | 页面控制台里 `window.__TAURI__` 为 `undefined` |
| CSP | `default-src 'self'; img-src 'self' asset: data:; media-src 'self' asset:; style-src 'self' 'unsafe-inline'` | 页面无任何外部请求 |
| 右键与刷新 | 全局禁用 `contextmenu`、`F5`、`Ctrl+R` | 手工按键验证 |
| 选择与拖拽 | `user-select: none`、`-webkit-user-drag: none` | 手工验证 |
| WebView2 遮挡检测 | 启动时设 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--disable-features=CalculateNativeWinOcclusion` | 熄屏 15 分钟唤醒后 1 秒内重绘（PRD U-7） |

> 最后一项不是安全项，是观感项：屏幕关闭期间 DWM 会把 WebView 判为遮挡并暂停渲染，唤醒后先白屏再跳数，U-7 会直接不达标。

---

## 6. 状态机实现

PRD §5 的状态与转移表**原样迁入**，此处只补实现约定（M4 已落地八态与下列全部条目）：

1. **单一入口**：所有转移必须经过 `session::transition(event, elapsed_ms)`，函数内部校验合法性；非法转移返回 `E_STATE_ILLEGAL` 并记日志。
   - **起势两段式**：`Start` 只把状态推到 `Arming`（记录 `startedAtMs` 与 `armed_at_mono_ms`），3 秒后由 tick 线程发 `ArmFinished` 才进 `Locked` 并创建计时器；`Cancel` 只在 `Arming` 合法（F-2.2）。
   - **到点判定的前置条件必须是 `state == Locked && session.is_some() && elapsed >= target`**。漏了 `Locked` 前置，`Arming` 期间（计时器尚未启动）就会误结算；漏了 `session.is_some()`，`Idle` 下 remaining 天然为 0 也会误结算。
2. **副作用集中在 `teardown()`，且顺序固定不可调换**：任何离开 Locked 的路径（到点 / 答题 / 抄写 / 紧急 / 显示变化 / 崩溃恢复）都必须走同一个 `teardown()`：
   0. 清 `AppState.locked`（让置顶维持线程先退，否则它会在 unlock 之后把窗口重新置顶）；
   1. **不停止 tick/io 线程**（它们是进程级的，见 §1.3）；出关的落盘收尾一律通过队列表达：`IoJob::ClearSnapshot` 入队即"排在所有快照之后删除"；
   2. 卸载键盘钩子、停止抢焦点监听；
   3. 清电源状态（**同一线程** `ES_CONTINUOUS`）；
   4. 还原亮度，成功则删除 `display_baseline.json`；
   5. 销毁副屏窗口，主窗口退出 Lock 视图并恢复原生标题栏；
   6. **同步**追加 `records.jsonl`（出关记录在这一步落盘，失败走 `E_STORE_FAILED` 横幅）；`endedAtMs` 取**结束时刻的墙钟**，不用 `startedAt + elapsed`（后者在被改过时间后是错的）；
   7. 投递 `IoJob::ClearSnapshot` 删除 `session.json`。

   **禁止在分支里各写一套清理逻辑**（这是这类软件"漏还原导致系统状态残留"的常见来源）。
3. **状态与投影分离**：`Locked/Cool/Challenge/Copying/Confirming` 五种状态共用同一个全屏窗口与同一个 `Lock.vue`，仅 `payload` 不同。切换状态不重建窗口。投影只在 `runtime::publish` 一处构造，五个分支不得各写一套。
4. **到点是状态机的最高优先级**：`session::transition()` 在查表之前先判 `remaining <= 0`，命中则强制 `Settled(completed)`，不论当前处于哪个状态。这条规则只能有一处实现，不得在 `Cool` / `Challenge` 分支里各写一遍。
5. **成本参数随会话固化**：`cooldownBase` / `questionBase` 在 `Start` 时从 `config.json` 带入 `SessionInfo`，状态机因此不读文件；中途改配置不影响本次闭关。
6. **冷却用单调 elapsed 计时**：`cool_end_elapsed_ms` 与 `elapsed_ms` 同一坐标系，改系统时间不能缩短冷却。`session.json` 的 `cooldownUntilMs` 只是换算出来的展示值，不参与判定。
7. **题与答案只在 Rust**：抽题、选项打乱、判定都在 Rust；`state://changed` 的 `challenge` 载荷没有 `answer` 字段，`questionId` 只用于拒绝陈旧点击。

---

## 7. 持久化

| 文件 | 位置 | 写策略 | 容错 |
|---|---|---|---|
| `config.json` | `%APPDATA%\闭关模拟器\` | 原子写（tmp + rename + fsync）；含设置与 `recentQuotes`（最近 5 条评语，防重复） | 解析失败 → 用默认值并备份为 `.bad` |
| `records.jsonl` | 同上 | 追加写，每行一次 flush | 损坏行跳过并计数，界面提示 |
| `session.json` | 同上 | 每 15 s 原子写 | 存在即视为异常中断 |
| `bank/seed.json` | 同上 | 首次从内嵌资源复制 | 解析失败 → 回退内嵌，不覆盖用户文件 |
| `bank/stats.json` | 原子写 | 每次作答后（`IoJob::BankStats`，整表覆写） | 解析失败 → 当空表（只影响记忆衰减，可重建） |
| `bank/stats.json` | 同上 | 每次答题后原子写 | 丢失仅影响记忆衰减，可重建为空 |
| `quotes.json` | 同上 | 首次复制，用户可改 | 解析失败 → 内嵌兜底 |
| `display_baseline.json` | 同上 | 入关写、出关删 | 存在即视为亮度可能残留 → 启动时自动还原并告知 |

**时间戳口径**：所有落盘时间戳为 **Unix 毫秒**（`i64`，字段名 `*Ms`），不存 ISO 字符串；本地日期换算只发生在统计层与渲染层（PRD §8、`decisions/003`）。

**路径全部经 Tauri path API 取得**，源码里不得出现硬编码盘符（对应 AGENTS 红线"禁止硬编码绝对路径"）。

---

## 8. 测试策略

### 8.1 可自动化（Rust 单测）

| 目标 | 用例要点 |
|---|---|
| 成本公式 | n=1..6 的冷却/题数/抄写字数；上限封顶；n 非法值；`remaining < cooldown(n)` 时 `request_unlock` 被拒（F-4.5） |
| 计时 | 注入 FakeClock + FakePower：正常走、调快 +1h（elapsed 不变）、调慢 −10min（elapsed 不变）、传统睡眠 10min（elapsed +10min）、**Modern Standby 10min（monoDelta≈wallDelta，elapsed 只 +10min 不 +20min）**、`RESUMESUSPEND` 单发、二者叠加 |
| 抽题 | 记忆衰减命中、答对 30 天内不重复、**答错 30 分钟内不重复**、可抽 < 需要时三级退化、题库永不为空、同轮不重复、打乱后判定仍正确 |
| 判分 | 全对通过；错 1 题即失败且不再出后续题；越界 `choice` 返回 `E_INVALID_CHOICE`；任何响应体不含 `answer` |
| 抄写 | n≥3 全对进 `Copying`、n<3 直接 `Settled`；完全一致通过；少 1 字 / 多 1 字 / 错 1 字失败；全角半角不归一化（严格）；`Copying` 中 Esc 不产生转移 |
| 统计 | 有效日门槛 25 分钟、当日多次累加；**23:50 开始 30 分钟归属 `startedAtMs` 当日**；`accident`/`interrupted` 计入累计但不计有效日；`emergency` 两者都不计；断签后显示 0 |
| 状态机 | PRD §5.2 每条转移 + 每条非法转移；**`Cool`/`Challenge`/`Copying`/`Confirming` 中 `remaining≤0` 一律进 `Settled(completed)`**；`attempts` 单调递增、答错不清零 |
| 存储 | 原子写断电模拟（写 tmp 后不 rename 应读旧值）；JSON 损坏行跳过；`teardown` 先停 io 队列再删快照（无"删后又被写回"）；`config`/`records` 版本迁移与未知字段保留 |

**当前规模（2026-10-21）**：`cargo test` 123 条、`vitest` 17 条。M4 新增的覆盖点：成本公式 8 条（含基数钳制与 `attempt=0`）、题库 6 条（内嵌合法性、坏文件回退、同轮不重复、种子可复现）、问阵 8 条（打乱保答案、错题不推进、越界、载荷无 `answer`）、抄写 7 条（少字/多字/错字/半角标点）、状态机 24 条（含五态到点与全部非法转移）。

### 8.2 只能手工（必须附截图/勾选表，标注"需要你确认"）

| 项 | 操作 | 通过标准 |
|---|---|---|
| 拦键 | 逐个按 PRD F-3.3 清单 | 全部无效，音量键有效 |
| 组合键状态机 | Win+L / Win+E / Alt+Tab / Ctrl+Alt+Tab / Win+Ctrl+→ / 正常中英文打字与输入法切换 | 前五条按预期（Win+L 可用、其余被拦），最后一条完全不受影响 |
| 任务栏 | 闭关中点任务栏右键、点开始按钮、点任务栏缩略图 | 均点不到（被覆盖窗口压住）；若某机型压不住，如实记为已知局限 |
| 多屏 | 双屏 + 副屏为主屏两种排列 | 均被覆盖，副屏显示镜像 |
| 熄屏唤醒 | 静置 15 分钟 | 仍锁定，倒计时正确 |
| 拔屏 | 闭关中拔 HDMI | 2 s 内出关，记 accident |
| 改缩放 | 闭关中改 150% | 2 s 内出关（与拔屏同路径） |
| 杀进程 | 任务管理器结束 | 锁消失；重启不续锁 |
| 亮度 | 入关/出关 | 降亮生效，出关精确还原（≤5%） |
| 观感 | 暖白/暗色 + 1366×768/4K | 无溢出、无遮挡、字体不糊 |

### 8.3 前端

不做 E2E。`vitest` 只测纯函数（时间格式化、抄写对比的展示态）。界面靠截图 + 手工核对。

---

## 9. 构建与发布

```
pnpm install
pnpm tauri build          # → src-tauri/target/release/biguansim.exe
```

| 项 | 规格 |
|---|---|
| 产物形态 | **文件夹**：`biguansim.exe`（Rust 静态链接，前端与资源已内嵌），不打单文件、不打 MSI / NSIS |
| 打包目标 | `tauri.conf.json` 设 `"bundle": { "targets": [] }`，否则 Windows 上会默认产出 MSI/NSIS 安装包 |
| 分发 | 压缩为 `闭关模拟器-v0.1.0-win64.zip`，解压即用 |
| 版本号 | 单一来源 `src-tauri/tauri.conf.json` 的 `version`；`package.json` 由脚本同步；Rust 用 `CARGO_PKG_VERSION` |
| 前端产物 | 内嵌进 exe（Tauri 默认），不散落 |
| 资源 | `src-tauri/resources/` 内嵌：阵法图案、音效、种子题库、语料 |
| 符号 | 保留 `biguansim.pdb` 便于自己排查，但**不进 zip** |

---

## 10. 性能预算

| 指标 | 目标 | 测量方式 |
|---|---|---|
| 安装目录体积 | ≤ 15 MB | 属性查看 |
| 冷启动 | 首次 ≤ 3 s；二次起 ≤ 1.5 s | 手动计时 3 次取中位（首次含 WebView2 用户数据目录创建，低端机达不到 1.5 s） |
| 空闲 CPU | < 1% | 任务管理器观察 5 分钟 |
| 8 小时内存增长 | ≤ 30 MB | 开工前后对比 |
| 钩子回调耗时 | < 100 µs | `QueryPerformanceCounter` 采样 1000 次取 P99 |
| 计时误差 | ≤ 1 s / 8 h | 与手机秒表对照 |

---

## 11. 技术风险与缓解

| 风险 | 触发条件 | 缓解 |
|---|---|---|
| 钩子被系统静默摘除 | 回调超时/UI 卡顿 | 回调零耗时 + **60 s 周期重装（不是消息探针）** + 抢焦点兜底 |
| `SetForegroundWindow` 被前台锁定策略拒绝 | 特定交互序列 | 3 次失败降频重试；仍失败则记日志，不弹窗 |
| WebView2 缺失 | 精简版 Windows | 启动时检测并给出安装指引，不静默崩溃 |
| 亮度还原失败 | 显示器热插拔、驱动异常 | 基线落盘 + 下次启动自动还原并告知 + 设置页手动恢复 |
| DDC/CI 不支持的显示器 | 部分外接屏 | 静默跳过，不影响主流程 |
| 显示快照误判 | 显卡驱动重置 | 连续 2 次不一致才判定 |
| 杀软拦截 | 未知发布者 exe | 文件夹发布 + 不混淆 + 不自启 + 不改注册表 |
| 多屏 DPI 混合 | 2K+1080p | 每屏独立计算逻辑尺寸，Per-Monitor V2 清单 |
| 拦键状态机不达预期 | "放行 Win+L"与"拦截 Win+E"无法共存 | M3 第一天 spike；退到最小集合（Alt+Tab / Ctrl+Esc / Ctrl+Shift+Esc / Alt+F4），并把 Win 组合键写进 PRD §13 |
| Modern Standby 重复计时 | S0ix 机型待机后剩余时间一次跳减两倍 | 差分公式 `max(0, wallDelta - monoDelta)` + 目标机型实测确认 |
| 任务栏压不住 | 混合 DPI 或某些驱动下 topmost 失效 | `rcMonitor` 覆盖 + 1 s 重申 TOPMOST；仍失败则如实写进 PRD §13-8，不装作拦住了 |

---

## 12. 目录结构

```
biguansim/
├─ AGENTS.md
├─ README.md
├─ package.json
├─ pnpm-lock.yaml
├─ .npmrc                        # 如需要，锁定 registry / 镜像
├─ index.html
├─ tsconfig.json
├─ vite.config.ts
├─ .prettierrc / eslint.config.js
├─ rust-toolchain.toml
├─ .gitignore
├─ docs/
│  ├─ 闭关模拟器-PRD-v0.1.md
│  ├─ 闭关模拟器-TDD-v0.1.md
│  ├─ 附加文档/                   # 原始构想、可行性、讨论稿（可入库）
│  ├─ _scratch/                  # 临时截图与素材草稿（.gitignore）
│  ├─ plans/
│  └─ decisions/
├─ src/                          # Vue
│  ├─ main.ts
│  ├─ App.vue
│  ├─ lib/ipc.ts
│  ├─ stores/session.ts
│  ├─ views/{Setup,Lock,Settle,Records,Bank,Settings,Onboarding}.vue
│  ├─ components/{DialogBox,Ring,StatCard,OptionList,CopyInput}.vue
│  └─ styles/{theme.css,layout.css}
└─ src-tauri/
   ├─ Cargo.toml
   ├─ Cargo.lock                 # 必须入库
   ├─ tauri.conf.json            # bundle.targets = []
   ├─ build.rs
   ├─ icons/                     # 必需：tauri build 会读，缺失直接失败
   ├─ capabilities/default.json
   ├─ resources/{formations/,sounds/,bank/,quotes.json}
   └─ src/
      ├─ main.rs
      ├─ lib.rs
      ├─ commands.rs
      ├─ events.rs
      ├─ core/{session,timer,cost,bank,stats,quote,store}.rs
      └─ platform/{hook,overlay,focus,power,brightness,display,single_instance}.rs
```

---

## 13. 开发顺序（对应 PRD §16）

| 阶段 | 模块 | 可独立验证的产出 |
|---|---|---|
| M1 | `platform::overlay` + `core::timer` + `core::store` + `platform::power::watch`（只订阅挂起/恢复，不阻止睡眠）+ **拦键 spike（原在 M3，2026-10-19 前移）** | 能全屏锁住、多屏覆盖、计时准、杀进程不续锁、挂起/恢复事件进日志；spike 五项有明确结论 |
| M2 | `core::session` + `events` + `Lock.vue` + 钟鸣 | 到点自动解锁全流程 |
| M3 | `platform::hook`（复用 M1 的 `keyfilter`）、`focus/sleep/display`（**`brightness` 延后**，见 PRD A-14 注） | 快捷键失效（含单击 Win 不弹开始菜单）、抢回前台 ≤50 ms、`powercfg /requests` 见请求、拔屏 2 s 内出关 |
| M4 | `core::cost/bank` + `Challenge` 投影 + 抄写 | 冷却与成本递增全链路 |
| M5 | `core::{bank_stats,stats}` + 记录/题库/设置/引导页 | **30 题** + 语料 **90 条** + 抄写段 10 段 + 7 个页面（rev4 修正规模） |

> M1 结束就应能"真用"（能锁 25 分钟、能到点解、能杀进程不锁死）。**先能锁，再谈漂亮。**

---

## 14. 技术取舍（T-1~T-6）

> **状态：全部已确认采纳（2026-10-19）**，实现按"结论"列执行；右列为已否决的备选，不再回退讨论。

| # | 取舍 | 结论 | 备选（已否决，勿重提） |
|---|---|---|---|
| T-1 | 判定逻辑放 Rust（本文 §1.1 推翻讨论稿"业务在 TS"） | 已采纳 | 放 TS（开发快，但锁不可信） |
| T-2 | 副屏显示镜像而非可交互界面 | 已采纳 | 副屏纯色 + 大号倒计时（更省资源） |
| T-3 | 不吃 `WM_DISPLAYCHANGE`，改 1 s 轮询快照 | 已采纳 | 用窗口消息（需额外窗口，且误报多） |
| T-4 | 亮度基线落盘 | 已采纳 | 只存内存（崩溃后亮度残留无法自动恢复） |
| T-5 | 不隐藏任务栏 | 已采纳 | 调 AppBar 隐藏（更彻底，但属于系统侵入） |
| T-6 | 前端不写 E2E 测试 | 已采纳 | Playwright + tauri-driver（成本高于收益，单人项目） |

---

## 15. 对 PRD 的修订记录（R-1~R-13，已回写）

> **状态：R-1~R-5 于 2026-10-19 回写 PRD v0.1-rev1；R-6~R-13 于 2026-10-19 回写 PRD v0.1-rev2**。下表保留原始差异，供追溯"为什么改"。
> 核对点：PRD F-3.15 / F-3.16 / F-3.20 / F-3.21 / F-3.11 / §11 可移植 / A-12 / §5.1 / §5.2 / F-4.3 / F-5.x / F-6.2 / F-8.2 / F-9.4 / F-9.5 / T-3 / A-17 / A-18 / §13 / §15 均已更新。

| # | PRD 位置 | 原表述 | 已改为 | 原因 | 状态 |
|---|---|---|---|---|---|
| R-1 | F-3.15 / F-3.16 | 「偏差 > 3 分钟…取**较大值**扣减」 | 单调时钟 + 睡眠事件累加；偏差仅用于显示「不一致」 | 取较大值等于用户把系统时间调快就送他时间，与 A-9 自相矛盾 | ✅ 已回写 |
| R-2 | A-12 | 「2 秒内自动出关」 | 保留，补注「1 秒轮询 × 连续 2 次」实现口径 | 无实质变更，仅消除歧义 | ✅ 已回写 |
| R-3 | F-3.20 / F-3.21 | 「监听 WM_DISPLAYCHANGE」 | 改为「每 1 秒轮询显示器快照 + 连续 2 次不一致判定」 | 无窗口线程收不到该消息；且熄屏唤醒会误报 | ✅ 已回写 |
| R-4 | F-3.11 | 「外接屏走 DDC/CI」 | 补「不支持 DDC/CI 的显示器或转接头直接跳过，不阻断流程」 | 部分显示器/转接头无 DDC 通道 | ✅ 已回写 |
| R-5 | §11 可移植 | 「复制文件夹可运行」 | 补「依赖系统 WebView2，缺失给安装指引」 | 避免在纯净系统上"复制过去打不开" | ✅ 已回写 |
| R-6 | F-5.8 / §5.1 / §5.2 | 抄写只写在功能条目里，状态机无对应状态 | 新增 `Copying` 状态与三条转移（Challenge→Copying→Settled） | 原表 `Challenge --全对--> Settled` 让抄写无处安放，AGENTS §5 要求的状态机测试也写不出来 | ✅ 已回写 |
| R-7 | §5.2 | 只定义了 `Locked --剩余≤0--> Settled` | 增加全局规则：任意状态 `remaining ≤ 0` → `Settled(completed)` | "剩余 3 分钟点解开 → 冷却 5 分钟"必然卡在 Cool/Challenge 里出不来 | ✅ 已回写 |
| R-8 | F-4.5（新增） | 无 | `remaining < cooldown(n)` 时拒绝进入 Cool 并提示稍候到点 | 否则用户等完冷却发现已到点，体验与状态机双输 | ✅ 已回写 |
| R-9 | F-8.2 | 只写"答对 30 天不抽" | 补三级退化回退 + 答错题 30 分钟冷却 | 60 题全部答对后会出现"无题可问"，直接违反原则 1 | ✅ 已回写 |
| R-10 | F-5.5 / F-5.10 | 未定义答错是否公布答案 | 明确不公布，响应体一律不含 `answer` | 公布答案 + 答错可重抽 = 第二道防线失效 | ✅ 已回写 |
| R-11 | F-9.4 / F-9.5 | `accident` / `interrupted` 时长全部作废 | 计入累计、不计有效日 | 崩溃是软件责任，白扣 2 小时会导致卸载 | ✅ 已回写 |
| R-12 | F-6.2 | 「8 字」 | 「9 字」 | 原文实为 9 个字符 | ✅ 已回写 |
| R-13 | T-3 | 「每类 ≥40 条（合计 360），v0.1 可先各 20 条」 | v0.1 交付 180 条（3×3×20），v0.2 补到 360 条 | 两个数量口径互相矛盾 | ✅ 已回写 |
