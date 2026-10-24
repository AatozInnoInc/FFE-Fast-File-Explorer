#![cfg_attr(not(windows), allow(dead_code))]
//! How a DirectWrite cell matches the Name column.
//!
//! A focused selected row uses the highlight colors. `LVS_SHOWSELALWAYS` keeps
//! the row marked when focus is in the tree or the address bar, and the list
//! view then paints it with the button face. The font size is the list-view
//! font's em height in the render target's device-independent pixels. This
//! module does not paint and does not read a directory.

/// Which system colors a cell uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowColors {
    /// Not drawn as selected. Window background and window text.
    Window,
    /// Selected while the list has focus. Highlight and highlight text.
    Highlight,
    /// Selected after focus has left the list. Button face and button text.
    Inactive,
}

/// `marked` is `LVIS_SELECTED`. The selection is visible when the list has
/// focus, or when it does not and `show_always` (`LVS_SHOWSELALWAYS`) is set.
/// The second case is the inactive palette.
pub fn selection(marked: bool, focused: bool, show_always: bool) -> RowColors {
    if !marked || (!focused && !show_always) {
        return RowColors::Window;
    }
    if focused {
        RowColors::Highlight
    } else {
        RowColors::Inactive
    }
}

/// Negative `lfHeight` is the em height in pixels. A positive value is the
/// cell height and is measured from the font. Zero is not a size. `i32::MIN`
/// has no positive counterpart.
pub fn negative_em(lf_height: i32) -> Option<i32> {
    if lf_height < 0 {
        lf_height.checked_neg()
    } else {
        None
    }
}

/// Em height in pixels to a DirectWrite DIP size at the render target `dpi`.
/// At 96 DPI one DIP is one DC pixel, which is how the DC render target is set up.
pub fn dip_size(em_pixels: i32, dpi: f32) -> Option<f32> {
    if em_pixels <= 0 || dpi <= 0.0 {
        return None;
    }
    Some(em_pixels as f32 * 96.0 / dpi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfocused_selection_uses_the_inactive_colors() {
        assert_eq!(selection(true, false, true), RowColors::Inactive);
        assert_eq!(selection(true, true, true), RowColors::Highlight);
        assert_eq!(selection(true, true, false), RowColors::Highlight);
        assert_eq!(selection(true, false, false), RowColors::Window);
        assert_eq!(selection(false, true, true), RowColors::Window);
        assert_eq!(selection(false, false, false), RowColors::Window);
    }

    #[test]
    fn negative_logfont_height_is_the_em() {
        assert_eq!(negative_em(-12), Some(12));
        assert_eq!(negative_em(-18), Some(18));
        assert_eq!(negative_em(0), None);
        assert_eq!(negative_em(16), None);
        assert_eq!(negative_em(i32::MIN), None);
    }

    #[test]
    fn dip_size_follows_the_render_target() {
        assert_eq!(dip_size(12, 96.0), Some(12.0));
        let at_144 = dip_size(18, 144.0).expect("size");
        assert!((at_144 - 12.0).abs() < 0.001, "{at_144}");
        let at_192 = dip_size(12, 192.0).expect("size");
        assert!((at_192 - 6.0).abs() < 0.001, "{at_192}");
        assert_eq!(dip_size(0, 96.0), None);
        assert_eq!(dip_size(-12, 96.0), None);
        assert_eq!(dip_size(12, 0.0), None);
        assert_eq!(dip_size(12, -96.0), None);
    }
}
