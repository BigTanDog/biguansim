//! 按键拦截策略：**纯函数**状态机，不碰系统 API。
//!
//! 为什么不能只做"按 VK 查表 return 1"：Windows 的组合键不是独立按键。
//! 只吞 `Win` 会连带废掉要放行的 `Win+L`；不吞 `Win` 则 `Win+E/D/R/I/S/X` 全部照常生效。
//!
//! 把判定抽成纯函数的好处：拦截策略可以用普通单测覆盖（TDD §8.2 里"组合键状态机
//! 只能手工"因此缩小到只剩系统行为那一层——钩子是否真的吞掉了按键）。

pub const VK_TAB: u16 = 0x09;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_F4: u16 = 0x73;

/// 放行 `Win+L`（PRD F-3.4：允许锁屏离开）
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

/// 按键状态机的唯一判定入口。
///
/// 调用约定：钩子回调里拿到 `WM_KEYDOWN/WM_SYSKEYDOWN`（`is_down = true`）与
/// `WM_KEYUP/WM_SYSKEYUP`（`false`）时各调用一次，先更新状态再取判定结果。
pub fn decide(vk: u16, is_down: bool, state: &mut ModState) -> Verdict {
    // 1. 修饰键本身的按下/抬起必须先记账
    match vk {
        VK_LWIN => state.lwin = is_down,
        VK_RWIN => state.rwin = is_down,
        VK_MENU => state.alt = is_down,
        VK_CONTROL => state.ctrl = is_down,
        VK_SHIFT => state.shift = is_down,
        _ => {}
    }

    // 2. Win 键本身必须放行——吞掉它系统就认为 Win 从未按下，Win+L 也就不成立
    if vk == VK_LWIN || vk == VK_RWIN {
        return Verdict::Pass;
    }

    // 3. Win 按住期间：放行 L（Win+L），其余字母/功能键一律吞掉
    if state.any_win() {
        return if is_down && vk == VK_L {
            Verdict::Pass
        } else {
            Verdict::Swallow
        };
    }

    // 4. Alt 按住期间：吞 Tab / Esc / F4，其余放行（避免误伤 AltGr 输入特殊字符）
    if state.alt {
        return match vk {
            VK_TAB | VK_ESCAPE | VK_F4 => Verdict::Swallow,
            _ => Verdict::Pass,
        };
    }

    // 5. Ctrl 按住期间：吞 Esc（含 Ctrl+Shift+Esc 任务管理器）
    if state.ctrl && vk == VK_ESCAPE {
        return Verdict::Swallow;
    }

    // 6. 无修饰键或无命中：放行（音量键、输入法切换等都落在这里）
    Verdict::Pass
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

    #[test]
    fn win_down_itself_passes() {
        let mut st = ModState::default();
        assert_eq!(press(&mut st, VK_LWIN), Verdict::Pass);
        assert!(st.lwin);
    }

    #[test]
    fn win_l_passes() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        assert_eq!(press(&mut st, VK_L), Verdict::Pass);
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

    #[test]
    fn single_win_release_does_not_stay_stuck() {
        let mut st = ModState::default();
        press(&mut st, VK_LWIN);
        assert_eq!(release(&mut st, VK_LWIN), Verdict::Pass);
        assert!(!st.any_win());
        // 抬起后字母键恢复放行
        assert_eq!(press(&mut st, 0x41), Verdict::Pass);
    }

    #[test]
    fn alt_tab_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_TAB), Verdict::Swallow);
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

    #[test]
    fn alt_f4_is_swallowed() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        assert_eq!(press(&mut st, VK_F4), Verdict::Swallow);
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

    #[test]
    fn alt_gr_typing_is_not_broken() {
        let mut st = ModState::default();
        press(&mut st, VK_MENU);
        press(&mut st, VK_CONTROL);
        // AltGr = Ctrl+Alt，打特殊字符时必须放行
        assert_eq!(press(&mut st, 0x45), Verdict::Pass);
    }
}
