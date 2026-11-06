#![cfg_attr(not(windows), allow(dead_code))]
#![cfg_attr(not(test), allow(dead_code))]
//! Alt+D focuses the address bar.
//!
//! Nothing here calls Win32. [`focuses_address`] reads modifier state and a
//! virtual-key code. Alt+Left and Alt+Right remain back and forward.

/// Alt, Ctrl, and Shift as they stand for one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mods {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

/// Virtual-key code for D. Alt+Left and Alt+Right are `0x25` and `0x27`.
const VK_D: u16 = 0x44;

/// Alt+D with Ctrl up and Shift up. `virtual_key` is a Win32 VK code.
pub fn focuses_address(mods: Mods, virtual_key: u16) -> bool {
    mods.alt && !mods.ctrl && !mods.shift && virtual_key == VK_D
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u16 = 0x41;
    const VK_LEFT: u16 = 0x25;
    const VK_RIGHT: u16 = 0x27;

    fn mods(alt: bool, ctrl: bool, shift: bool) -> Mods {
        Mods { alt, ctrl, shift }
    }

    #[test]
    fn alt_d_focuses_the_address_bar() {
        assert!(focuses_address(mods(true, false, false), VK_D));
    }

    #[test]
    fn d_without_alt_does_not_focus() {
        assert!(!focuses_address(mods(false, false, false), VK_D));
    }

    #[test]
    fn ctrl_alt_d_does_not_focus() {
        assert!(!focuses_address(mods(true, true, false), VK_D));
    }

    #[test]
    fn alt_shift_d_does_not_focus() {
        assert!(!focuses_address(mods(true, false, true), VK_D));
    }

    #[test]
    fn alt_left_stays_back() {
        assert!(!focuses_address(mods(true, false, false), VK_LEFT));
    }

    #[test]
    fn alt_right_stays_forward() {
        assert!(!focuses_address(mods(true, false, false), VK_RIGHT));
    }

    #[test]
    fn alt_with_another_key_does_not_focus() {
        assert!(!focuses_address(mods(true, false, false), VK_A));
    }
}
