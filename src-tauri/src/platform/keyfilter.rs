//! 按键拦截策略：**纯函数**状态机，不碰系统 API。
//!
//! 为什么不能只做"按 VK 查表 return 1"：Windows 的组合键不是独立按键。
//! `Win` 的按下与抬起**都吞掉**：系统完全看不到这个键，既不会弹开始菜单，也不会残留
//! "Win 一直按住"（后者会让后续按键变成 `Win+X`，实测按 s/d 直接开出搜索、翻出桌面）。
//! 吞掉 `Win+L` 后由 hook 线程调 `LockWorkStation()` 代劳锁屏（F-3.4）。
//!
//! 把判定抽成纯函数的好处：拦截策略可以用普通单测覆盖（TDD §8.2 里"组合键状态机
//! 只能手工"因此缩小到只剩系统行为那一层——钩子是否真的吞掉了按键）。

pub const VK_TAB: u16 = 0x09;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_F4: u16 = 0x73;
pub const VK_F11: u16 = 0x7A;

/// 左右专用修饰键：低级钩子可能上报通用 VK，也可能上报这些（实测两者都会出现）
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// `Win+L` 的 L
const VK_L: u16 = 0x4C;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModState {
    pub lwin: bool,
    pub rwin: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl ModState {
    pub fn any_win(&self) -> bool {
        self.lwin || self.rwin
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 吞掉：`return 1`，按键不下传
    Swallow,
    /// 放行：`CallNextHookEx`
    Pass,
}

fn is_ctrl(vk: u16) -> bool {
    matches!(vk, VK_CONTROL | VK_LCONTROL | VK_RCONTROL)
}

fn is_shift(vk: u16) -> bool {
    matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT)
}

fn is_alt(vk: u16) -> bool {
    matches!(vk, VK_MENU | VK_LMENU | VK_RMENU)
}

/// 按键状态机的唯一判定入口。
///
/// 调用约定：钩子回调里拿到 `WM_KEYDOWN/WM_SYSKEYDOWN`（`is_down = true`）与
/// `WM_KEYUP/WM_SYSKEYUP`（`false`）时各调用一次，先更新状态再取判定结果。
pub fn decide(vk: u16, is_down: bool, state: &mut ModState) -> Verdict {
    // 1. 修饰键本身先记账（通用 VK 与左右专用 VK 都要认）
    match vk {
        VK_LWIN => state.lwin = is_down,
        VK_RWIN => state.rwin = is_down,
        _ if is_alt(vk) => state.alt = is_down,
        _ if is_ctrl(vk) => state.ctrl = is_down,
        _ if is_shift(vk) => state.shift = is_down,
        _ => {}
    }

    // 1.5 开发期逃生组合键：显式放行，且**排在所有吞键分支之前**。
    // 只在 debug 构建存在；release 里这段代码根本编不进去。
    // 为什么单独列一条而不是"靠分支自然放行"：`Ctrl+Alt+Shift+X` 里的 Alt+Shift
    // 是系统的输入法切换热键，可能先被系统吃掉（2026-10-20 实测失效，原因未定论），
    // 换成不含 Alt 的 `Ctrl+Shift+F12` 可绕开；两条都保留。
    #[cfg(debug_assertions)]
    if dev_exit_combo(vk, state) {
        return Verdict::Pass;
    }

    // 2. Win 键：按下与抬起都吞掉。
    //
    // 这里踩过一个把键盘弄"半死"的坑（2026-10-20 实测）：早先"按下放行、抬起吞掉"
    // 想让开始菜单别弹，结果系统认为 Win 一直按住——之后每次按键都被解释成 Win+X，
    // 按 s/d 直接触发系统快捷键甚至显示桌面。现在两头都吞，系统完全没见过 Win；
    // `Win+L` 由 hook 线程调 `LockWorkStation()` 代劳，语义不变。
    if vk == VK_LWIN || vk == VK_RWIN {
        // 吞"按下"、放"抬起"，这是唯一同时满足两件事的组合（2026-10-20 实测对照）：
        // - 吞下：开始菜单/搜索不弹（吞下 + 放上，前台停在原窗口）；
        // - 放上：让系统清掉 Win 的按下状态；若连抬起一起吞，系统会认为 Win 一直按住，
        //   之后每个键都变成 Win+X（实测按 s/d 直接开出搜索与桌面）。
        return Verdict::Swallow;
    }

    // 3. Win 按住期间：字母与功能键一律吞掉（含 L，锁屏由 hook 代劳）
    if state.any_win() {
        return Verdict::Swallow;
    }

    // 4. Alt 按住期间：吞 Tab / Esc / F4 / Space（Alt+Space 是系统菜单，能把窗口最小化）
    if state.alt {
        return match vk {
            VK_TAB | VK_ESCAPE | VK_F4 | VK_SPACE => Verdict::Swallow,
            _ => Verdict::Pass,
        };
    }

    // 5. Ctrl 按住期间：吞 Esc（含 Ctrl+Shift+Esc 任务管理器）
    if state.ctrl && vk == VK_ESCAPE {
        return Verdict::Swallow;
    }

    // 6. 无修饰键：F11（F-3.3 清单里有）要吞，其余放行（音量键、输入法切换等）
    if vk == VK_F11 {
        return Verdict::Swallow;
    }

    Verdict::Pass
}

/// 开发期逃生组合键（**只在 debug 构建**）：`Ctrl+Shift+F12` 或 `Ctrl+Alt+Shift+X`。
#[cfg(debug_assertions)]
fn dev_exit_combo(vk: u16, state: &ModState) -> bool {
    const VK_X: u16 = 0x58;
    const VK_F12: u16 = 0x7B;
    (vk == VK_F12 && state.ctrl && state.shift)
        || (vk == VK_X && state.ctrl && state.alt && state.shift)
}

/// Esc 长按计时。
///
/// **踩过的坑（2026-10-21 实测）**：原来每次 `WM_KEYDOWN` 都把起点改成"现在"，
/// 但长按 Esc 时系统每 ~30 ms 发一次自动重复的 keydown——起点被无限刷新，
/// `esc_hold_ms()` 永远接近 0，8 秒永远到不了，界面上连进度环都看不见。
/// 所以必须区分"按下沿"与"自动重复"：只有**未 armed 的按下**才记起点。
///
/// 位布局（塞进一个 `AtomicU64`，因为钩子回调里不许加锁、不许分配）：
/// bit0 = 物理按住，bit1 = 允许计时（armed），高位 = 按下时刻（毫秒）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EscBits(u64);

const ESC_HELD: u64 = 1;
const ESC_ARMED: u64 = 1 << 1;

impl EscBits {
    pub const IDLE: EscBits = EscBits(0);

    pub fn from_raw(raw: u64) -> EscBits {
        EscBits(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    pub fn is_held(self) -> bool {
        self.0 & ESC_HELD != 0
    }

    pub fn is_armed(self) -> bool {
        self.0 & ESC_ARMED != 0
    }

    /// 按下时刻（毫秒）；未计时时为 0
    pub fn down_ms(self) -> u64 {
        self.0 >> 2
    }
}

/// 一次 Esc 按键事件 → 新的计时状态。`is_down` 对自动重复也是 true，
/// 靠 armed 位把重复事件挡在外面。
pub fn esc_on_key(state: EscBits, is_down: bool, now_ms: u64) -> EscBits {
    if !is_down {
        return EscBits::IDLE;
    }
    if state.is_held() {
        // 自动重复（armed 与否都一样）：保持原状态，绝不刷新起点，
        // 也绝不把"被消费过的按住"重新武装——否则取消后按着不放会立刻二次触发。
        return state;
    }
    // 真正的按下沿：`now_ms.max(1)` 保证 down_ms 不为 0（0 会被当成"未计时"）
    EscBits((now_ms.max(1) << 2) | ESC_HELD | ESC_ARMED)
}

/// 已按住的毫秒数；没有 armed 的按下（含被消费过的、只剩物理按住的）返回 0。
pub fn esc_hold_ms(state: EscBits, now_ms: u64) -> u64 {
    if !state.is_armed() || state.down_ms() == 0 {
        return 0;
    }
    now_ms.saturating_sub(state.down_ms())
}

/// 触发或取消破戒后调用：清掉计时，但保留"物理仍按住"这一事实。
/// 这样按住不放不会立刻二次触发，必须先松开再按下（F-6.4 的进度清零）。
pub fn esc_consume(state: EscBits) -> EscBits {
    if state.is_held() {
        EscBits(ESC_HELD)
    } else {
        EscBits::IDLE
    }
}

/// 是否要替系统执行"锁屏"：`Win+L` 被我们整体吞掉后，由 hook 线程主动调用
/// `LockWorkStation()`，这样既不会露开始菜单，F-3.4 的"允许锁屏离开"也还在。
pub fn wants_lock(vk: u16, is_down: bool, win_down: bool) -> bool {
    is_down && win_down && vk == VK_L
}

/// 用真实键态校正状态机（`GetAsyncKeyState` 的结果）。
///
/// 为什么必须有：锁屏/切安全桌面时，按键的抬起事件不会回到我们这里，
/// `lwin` 会永远停在 true，解锁后所有按键都被当成 Win 组合吞掉。
/// 只改五个修饰位。
pub fn reconcile(state: &mut ModState, lwin: bool, rwin: bool, alt: bool, ctrl: bool, shift: bool) {
    state.lwin = lwin;
    state.rwin = rwin;
    state.alt = alt;
    state.ctrl = ctrl;
    state.shift = shift;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(state: &mut ModState, vk: u16) -> Verdict {
        decide(vk, true, state)
    }

    fn release(state: &mut ModState, vk: u16) -> Verdict {
        decide(vk, false, state)
    }

    /// 按下与抬起都吞掉：系统完全看不到 Win，既不会弹开始菜单，
    /// 也不会留下"Win 一直按住"的状态（后者正是"键盘半死"的根因）
    #[test]
    fn win_down_and_up_are_both_swallowed() {
        let mut st = ModState::default();
        assert_eq!(press(&mut st, VK_LWIN), Verdict::Swallow);
        assert!(st.lwin);

        assert_eq!(release(&mut st, VK_LWIN), Verdict::Swallow);
        assert!(!st.any_win(), "抬起后不得残留 Win 按住状态");
    }

    /// 单击 Win 之后，普通打字必须恢复正常
    #[test]
    fn typing_after_win_tap_passes() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        release(&mut st, VK_LWIN);

        assert_eq!(press(&mut st, 0x41), Verdict::Pass, "a");
        assert_eq!(press(&mut st, 0x53), Verdict::Pass, "s");
        assert_eq!(press(&mut st, 0x44), Verdict::Pass, "d");
    }

    /// Win+L 被吞掉，但要向系统发出"锁屏"请求（F-3.4）
    #[test]
    fn win_l_is_swallowed_and_requests_lock() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);

        assert_eq!(press(&mut st, VK_L), Verdict::Swallow);
        assert!(wants_lock(VK_L, true, true));
        assert!(!wants_lock(VK_L, false, true), "抬起不应再触发锁屏");
        assert!(!wants_lock(VK_L, true, false), "没按 Win 时 L 就是普通字母");
    }

    #[test]
    fn win_e_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        assert_eq!(press(&mut st, 0x45), Verdict::Swallow);
    }

    #[test]
    fn win_d_r_i_s_x_are_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        for vk in [0x44u16, 0x52, 0x49, 0x53, 0x58] {
            assert_eq!(press(&mut st, vk), Verdict::Swallow, "VK {vk:#X} 应被吞");
        }
    }

    /// 锁屏回来后按键抬起事件丢失、状态卡在 "Win 按住" —— 必须能靠真实键态纠正
    #[test]
    fn reconcile_clears_stuck_win() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        assert!(st.lwin);

        reconcile(&mut st, false, false, false, false, false);
        assert!(!st.any_win(), "真实键态为松开时必须清掉");
        assert_eq!(press(&mut st, 0x41), Verdict::Pass, "纠正后字母应能打出来");
    }

    #[test]
    fn alt_tab_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_TAB), Verdict::Swallow);
    }

    /// Alt+Space 会拉起系统菜单并把窗口最小化，必须一起吞
    #[test]
    fn alt_space_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_SPACE), Verdict::Swallow);
    }

    #[test]
    fn ctrl_alt_tab_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_TAB), Verdict::Swallow);
    }

    #[test]
    fn ctrl_escape_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        assert_eq!(press(&mut st, VK_ESCAPE), Verdict::Swallow);
    }

    #[test]
    fn ctrl_shift_escape_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        press(&mut st, VK_SHIFT);
        assert_eq!(press(&mut st, VK_ESCAPE), Verdict::Swallow);
    }

    /// 低级钩子上报的是左右专用 VK 时，同样要认得出 Ctrl/Alt/Shift
    #[test]
    fn side_specific_modifier_vks_are_recognised() {
        let mut st = ModState::default();
        press(&mut st, VK_LCONTROL);
        press(&mut st, VK_LSHIFT);
        assert_eq!(
            press(&mut st, VK_ESCAPE),
            Verdict::Swallow,
            "Ctrl+Shift+Esc"
        );

        let mut st = ModState::default();
        press(&mut st, VK_LMENU);
        assert_eq!(press(&mut st, VK_TAB), Verdict::Swallow, "Alt+Tab");
    }

    #[test]
    fn alt_f4_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_F4), Verdict::Swallow);
    }

    /// F-3.3 清单里有 F11（全屏切换），无修饰键时也要吞
    #[test]
    fn f11_is_swallowed() {
        let mut st = ModState::default();
        assert_eq!(press(&mut st, VK_F11), Verdict::Swallow);
    }

    #[test]
    fn win_ctrl_arrow_is_swallowed_for_virtual_desktops() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        press(&mut st, VK_CONTROL);
        assert_eq!(press(&mut st, 0x25), Verdict::Swallow);
        assert_eq!(press(&mut st, 0x27), Verdict::Swallow);
    }

    #[test]
    fn left_right_win_do_not_share_state() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        press(&mut st, VK_RWIN);
        release(&mut st, VK_LWIN);
        assert!(!st.lwin);
        assert!(st.rwin);

        // 右 Win 仍按住，字母键仍应被吞
        assert_eq!(press(&mut st, 0x45), Verdict::Swallow);
    }

    #[test]
    fn plain_keys_and_volume_pass_through() {
        let mut st = ModState::default();
        for vk in [0x41u16, 0x30, 0x20, 0xAD, 0xAE, 0x14] {
            assert_eq!(press(&mut st, vk), Verdict::Pass, "VK {vk:#X} 应放行");
        }
    }

    #[test]
    fn ime_switch_keys_pass_through() {
        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        // Ctrl+Space 是输入法切换，不能误伤
        assert_eq!(press(&mut st, 0x20), Verdict::Pass);
        release(&mut st, VK_CONTROL);
        assert_eq!(press(&mut st, VK_SHIFT), Verdict::Pass);
    }

    /// 开发期逃生组合键必须放行：否则闭关中无法退出，只能杀进程
    #[cfg(debug_assertions)]
    #[test]
    fn dev_exit_combo_passes_through() {
        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        press(&mut st, VK_SHIFT);
        assert_eq!(press(&mut st, 0x7B), Verdict::Pass, "Ctrl+Shift+F12");

        let mut st = ModState::default();
        press(&mut st, VK_CONTROL);
        press(&mut st, VK_MENU);
        press(&mut st, VK_SHIFT);
        assert_eq!(press(&mut st, 0x58), Verdict::Pass, "Ctrl+Alt+Shift+X");

        // 只有 F12 单独按（不带修饰键）仍按普通键处理，不引入额外吞键
        let mut st = ModState::default();
        assert_eq!(press(&mut st, 0x7B), Verdict::Pass, "F12");
    }

    // ---------- Esc 长按计时（2026-10-21 真机踩坑的回归） ----------

    /// 回归：长按 Esc 时系统每 ~30 ms 发一次自动重复 keydown。
    /// 旧实现每次重复都刷新起点，导致 hold 永远接近 0、8 秒永远不到。
    #[test]
    fn esc_auto_repeat_does_not_restart_the_timer() {
        let mut state = esc_on_key(EscBits::IDLE, true, 1_000);
        assert_eq!(esc_hold_ms(state, 1_030), 30);

        // 连续 20 次自动重复（每 30 ms 一次）
        for tick in 1..=20u64 {
            state = esc_on_key(state, true, 1_000 + tick * 30);
        }

        assert_eq!(
            esc_hold_ms(state, 1_600),
            600,
            "自动重复不得刷新起点，否则永远到不了 8 秒"
        );
        assert_eq!(esc_hold_ms(state, 9_000), 8_000);
    }

    #[test]
    fn esc_hold_grows_while_held() {
        let state = esc_on_key(EscBits::IDLE, true, 5_000);
        assert_eq!(esc_hold_ms(state, 5_000), 0);
        assert_eq!(esc_hold_ms(state, 7_500), 2_500);
        assert_eq!(esc_hold_ms(state, 8_000), 3_000);
    }

    #[test]
    fn esc_release_clears_everything() {
        let state = esc_on_key(EscBits::IDLE, true, 1_000);
        let released = esc_on_key(state, false, 2_000);

        assert_eq!(released, EscBits::IDLE);
        assert_eq!(esc_hold_ms(released, 9_000), 0);
        assert!(!released.is_held());
    }

    /// F-6.4：取消破戒后进度清零；按住不放不得立刻二次触发，必须松开重按
    #[test]
    fn esc_consume_requires_a_fresh_press() {
        let held = esc_on_key(EscBits::IDLE, true, 1_000);
        let consumed = esc_consume(held);

        assert!(consumed.is_held(), "物理上仍按住");
        assert!(!consumed.is_armed());
        assert_eq!(esc_hold_ms(consumed, 10_000), 0, "消费后不得继续累计");

        // 仍按住时的自动重复也不能把它重新启动
        let repeat = esc_on_key(consumed, true, 10_030);
        assert_eq!(esc_hold_ms(repeat, 18_000), 0);

        // 松开再按下 → 重新从 0 开始计时
        let released = esc_on_key(repeat, false, 10_100);
        let pressed = esc_on_key(released, true, 11_000);
        assert_eq!(esc_hold_ms(pressed, 11_500), 500);
    }

    #[test]
    fn esc_hold_without_press_is_zero() {
        assert_eq!(esc_hold_ms(EscBits::IDLE, 9_999), 0);
    }

    #[test]
    fn alt_gr_typing_is_not_broken() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        press(&mut st, VK_CONTROL);
        // AltGr = Ctrl+Alt，打特殊字符时必须放行
        assert_eq!(press(&mut st, 0x45), Verdict::Pass);
    }
}
