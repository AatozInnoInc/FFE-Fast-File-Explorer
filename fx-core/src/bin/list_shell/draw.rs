//! DirectWrite for one list-view cell.
//!
//! The factories are created once on the UI thread. A failed factory leaves
//! the caller to paint the dispinfo string. Painting does not read a directory
//! and does not wait.

use std::cell::RefCell;

use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_sys::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
};

/// `D2DERR_RECREATE_TARGET`. The DC target must be built again after this.
const RECREATE_TARGET: i32 = 0x8899_000C_u32 as i32;

/// A cell rectangle in the list view's client coordinates.
#[derive(Clone, Copy)]
pub struct CellRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

struct Painter {
    factory: ID2D1Factory,
    format: IDWriteTextFormat,
    target: Option<ID2D1DCRenderTarget>,
    /// Creating the DC target failed. Later cells keep the list view's text.
    target_broken: bool,
}

enum Slot {
    Untried,
    Failed,
    Ready(Painter),
    Busy,
}

thread_local! {
    static PAINTER: RefCell<Slot> = const { RefCell::new(Slot::Untried) };
}

/// Fills `bounds` and draws `text` with DirectWrite. False means the list view
/// should paint the string itself.
pub fn paint_text(
    hdc: *mut core::ffi::c_void,
    bounds: CellRect,
    text_bounds: CellRect,
    text: &str,
    selected: bool,
) -> bool {
    if hdc.is_null() || text.is_empty() || !positive(bounds) {
        return false;
    }
    PAINTER.with(|slot| {
        let current = slot.replace(Slot::Busy);
        match current {
            Slot::Busy => false,
            Slot::Failed => {
                *slot.borrow_mut() = Slot::Failed;
                false
            }
            Slot::Untried => match Painter::create() {
                Ok(painter) => finish(slot, painter, hdc, bounds, text_bounds, text, selected),
                Err(()) => {
                    *slot.borrow_mut() = Slot::Failed;
                    false
                }
            },
            Slot::Ready(painter) => finish(slot, painter, hdc, bounds, text_bounds, text, selected),
        }
    })
}

fn finish(
    slot: &RefCell<Slot>,
    mut painter: Painter,
    hdc: *mut core::ffi::c_void,
    bounds: CellRect,
    text_bounds: CellRect,
    text: &str,
    selected: bool,
) -> bool {
    let ok = painter.paint(hdc, bounds, text_bounds, text, selected);
    *slot.borrow_mut() = Slot::Ready(painter);
    ok
}

impl Painter {
    fn create() -> Result<Self, ()> {
        let factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                .map_err(|_| ())?;
        let write: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.map_err(|_| ())?;
        let format = text_format(&write)?;
        let target = create_target(&factory).ok();
        Ok(Self {
            factory,
            format,
            target_broken: target.is_none(),
            target,
        })
    }

    fn paint(
        &mut self,
        hdc: *mut core::ffi::c_void,
        bounds: CellRect,
        text_bounds: CellRect,
        text: &str,
        selected: bool,
    ) -> bool {
        if self.ensure_target().is_err() {
            return false;
        }
        let Some(target) = self.target.clone() else {
            return false;
        };
        let text_bounds = text_area(bounds, text_bounds);
        let bound = windows::Win32::Foundation::RECT {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        if unsafe { target.BindDC(HDC(hdc), &bound) }.is_err() {
            return false;
        }
        unsafe { target.BeginDraw() };
        let drew = draw_cell(&target, &self.format, bounds, text_bounds, text, selected);
        match unsafe { target.EndDraw(None, None) } {
            Ok(()) => drew,
            Err(err) if err.code().0 == RECREATE_TARGET => {
                self.target = None;
                self.target_broken = false;
                false
            }
            Err(_) => false,
        }
    }

    fn ensure_target(&mut self) -> Result<(), ()> {
        if self.target.is_some() {
            return Ok(());
        }
        if self.target_broken {
            return Err(());
        }
        match create_target(&self.factory) {
            Ok(target) => {
                self.target = Some(target);
                Ok(())
            }
            Err(()) => {
                self.target_broken = true;
                Err(())
            }
        }
    }
}

fn draw_cell(
    target: &ID2D1DCRenderTarget,
    format: &IDWriteTextFormat,
    bounds: CellRect,
    text_bounds: CellRect,
    text: &str,
    selected: bool,
) -> bool {
    let (background, foreground) = if selected {
        (COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT)
    } else {
        (COLOR_WINDOW, COLOR_WINDOWTEXT)
    };
    let background = color_f(unsafe { GetSysColor(background) });
    let foreground = color_f(unsafe { GetSysColor(foreground) });
    let Ok(fill) = (unsafe { target.CreateSolidColorBrush(&background, None) }) else {
        return false;
    };
    let Ok(ink) = (unsafe { target.CreateSolidColorBrush(&foreground, None) }) else {
        return false;
    };
    let area = d2_rect(bounds);
    unsafe { target.FillRectangle(&area, &fill) };
    let units: Vec<u16> = text.encode_utf16().collect();
    if !units.is_empty() {
        let layout = d2_rect(text_bounds);
        unsafe {
            target.DrawText(
                &units,
                format,
                &layout,
                &ink,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    true
}

fn text_format(write: &IDWriteFactory) -> Result<IDWriteTextFormat, ()> {
    for family in [w!("Segoe UI"), w!("Arial")] {
        let Ok(format) = (unsafe {
            write.CreateTextFormat(
                family,
                None::<&IDWriteFontCollection>,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                w!("en-us"),
            )
        }) else {
            continue;
        };
        let _ = unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING) };
        let _ = unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
        let _ = unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) };
        return Ok(format);
    }
    Err(())
}

fn create_target(factory: &ID2D1Factory) -> Result<ID2D1DCRenderTarget, ()> {
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 0.0,
        dpiY: 0.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let target = unsafe { factory.CreateDCRenderTarget(&properties) }.map_err(|_| ())?;
    unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    Ok(target)
}

fn color_f(colorref: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (colorref & 0xff) as f32 / 255.0,
        g: ((colorref >> 8) & 0xff) as f32 / 255.0,
        b: ((colorref >> 16) & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn d2_rect(rect: CellRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left as f32,
        top: rect.top as f32,
        right: rect.right as f32,
        bottom: rect.bottom as f32,
    }
}

fn positive(rect: CellRect) -> bool {
    rect.right > rect.left && rect.bottom > rect.top
}

fn text_area(bounds: CellRect, text_bounds: CellRect) -> CellRect {
    if positive(text_bounds) {
        text_bounds
    } else {
        CellRect {
            left: bounds.left.saturating_add(4),
            top: bounds.top,
            right: bounds.right.saturating_sub(2).max(bounds.left),
            bottom: bounds.bottom,
        }
    }
}
