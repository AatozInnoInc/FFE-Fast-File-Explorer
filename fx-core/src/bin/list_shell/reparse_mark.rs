#![cfg_attr(not(windows), allow(dead_code))]
//! Geometry for the reparse badge on the Name-column icon.
//!
//! The badge is a square in the bottom-right of the icon when the entry is a
//! reparse point. Its side is one third of the shorter icon edge, at least 4
//! pixels, and it has to fit inside the icon. An icon thinner than 8 pixels
//! on either edge has no badge. This module does not load an overlay handler,
//! and it does not read a directory.

/// A rectangle in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Square badge in the bottom-right of the icon.
///
/// The side is `max(4, min(width, height) / 3)`, with integer division. The
/// badge's right and bottom edges sit on the icon's. [`None`] when the width
/// or the height is below 8, when `right <= left` or `bottom <= top`, when
/// that difference does not fit in `i32`, or when the side does not fit.
pub fn reparse_badge(left: i32, top: i32, right: i32, bottom: i32) -> Option<PxRect> {
    let width = match right.checked_sub(left) {
        Some(width) if width > 0 => width,
        _ => return None,
    };
    let height = match bottom.checked_sub(top) {
        Some(height) if height > 0 => height,
        _ => return None,
    };
    if width < 8 || height < 8 {
        return None;
    }
    let side = 4.max(width.min(height) / 3);
    if side > width || side > height {
        return None;
    }
    Some(PxRect {
        left: right - side,
        top: bottom - side,
        right,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_16_icon_gets_a_side_of_5_flush_to_the_corner() {
        let badge = reparse_badge(0, 0, 16, 16).expect("badge");
        assert_eq!(badge.right - badge.left, 5);
        assert_eq!(badge.bottom - badge.top, 5);
        assert_eq!(badge.right, 16);
        assert_eq!(badge.bottom, 16);
        assert_eq!(
            badge,
            PxRect {
                left: 11,
                top: 11,
                right: 16,
                bottom: 16,
            }
        );
    }

    #[test]
    fn an_8_icon_gets_a_side_of_4() {
        assert_eq!(
            reparse_badge(0, 0, 8, 8),
            Some(PxRect {
                left: 4,
                top: 4,
                right: 8,
                bottom: 8,
            })
        );
    }

    #[test]
    fn a_width_below_8_has_no_badge() {
        assert_eq!(reparse_badge(0, 0, 7, 16), None);
        assert_eq!(reparse_badge(0, 0, 16, 7), None);
    }

    #[test]
    fn an_inverted_rect_has_no_badge() {
        assert_eq!(reparse_badge(16, 0, 0, 16), None);
        assert_eq!(reparse_badge(0, 16, 16, 0), None);
        assert_eq!(reparse_badge(8, 0, 8, 16), None);
        assert_eq!(reparse_badge(0, 8, 16, 8), None);
        assert_eq!(reparse_badge(i32::MAX, 0, i32::MIN, 16), None);
        assert_eq!(reparse_badge(i32::MIN, 0, i32::MAX, 16), None);
    }

    #[test]
    fn a_wide_30_by_10_icon_gets_a_side_of_4() {
        assert_eq!(
            reparse_badge(0, 0, 30, 10),
            Some(PxRect {
                left: 26,
                top: 6,
                right: 30,
                bottom: 10,
            })
        );
    }
}
