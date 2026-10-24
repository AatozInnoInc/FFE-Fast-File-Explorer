//! DirectWrite for one list-view cell.
//!
//! The factories are created once on the UI thread. The text format is the
//! list view's own font, and the colors follow [`crate::cell::selection`].
//! A failed factory or a font DirectWrite cannot build leaves the caller to
//! paint the dispinfo string. Painting does not read a directory and does not
//! wait.

use std::cell::RefCell;
use std::mem;
use std::ptr;

use windows::core::{w, PCWSTR};
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
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, GetObjectW, GetSysColor, GetTextMetricsW, SelectObject,
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, LOGFONTW, TEXTMETRICW,
};

use crate::cell::{dip_size, negative_em, RowColors};

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
    write: IDWriteFactory,
    format: Option<(FontKey, IDWriteTextFormat)>,
    /// This list-view font could not be built. Later cells with the same font
    /// stay with the control instead of trying again.
    failed: Option<FontKey>,
    target: Option<ID2D1DCRenderTarget>,
    /// Creating the DC target failed. Later cells keep the list view's text.
    target_broken: bool,
}

/// The list-view font, reduced to what DirectWrite needs for one text format.
/// `height` is `lfHeight`. The em size is derived from it when the format is built.
#[derive(PartialEq, Eq)]
struct FontKey {
    face: [u16; 32],
    height: i32,
    weight: i32,
    italic: u8,
    dpi_bits: u32,
}

struct Request<'a> {
    hdc: *mut core::ffi::c_void,
    font: *mut core::ffi::c_void,
    bounds: CellRect,
    text_bounds: CellRect,
    text: &'a str,
    colors: RowColors,
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
/// should paint the string itself. `font` is the list view's `WM_GETFONT`.
pub fn paint_text(
    hdc: *mut core::ffi::c_void,
    font: *mut core::ffi::c_void,
    bounds: CellRect,
    text_bounds: CellRect,
    text: &str,
    colors: RowColors,
) -> bool {
    if hdc.is_null() || font.is_null() || text.is_empty() || !positive(bounds) {
        return false;
    }
    let request = Request {
        hdc,
        font,
        bounds,
        text_bounds,
        text,
        colors,
    };
    PAINTER.with(|slot| {
        let current = slot.replace(Slot::Busy);
        match current {
            Slot::Busy => false,
            Slot::Failed => {
                *slot.borrow_mut() = Slot::Failed;
                false
            }
            Slot::Untried => match Painter::create() {
                Ok(painter) => finish(slot, painter, &request),
                Err(()) => {
                    *slot.borrow_mut() = Slot::Failed;
                    false
                }
            },
            Slot::Ready(painter) => finish(slot, painter, &request),
        }
    })
}

fn finish(slot: &RefCell<Slot>, mut painter: Painter, request: &Request<'_>) -> bool {
    let ok = painter.paint(request);
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
        let target = create_target(&factory).ok();
        Ok(Self {
            factory,
            write,
            format: None,
            failed: None,
            target_broken: target.is_none(),
            target,
        })
    }

    fn paint(&mut self, request: &Request<'_>) -> bool {
        if self.ensure_target().is_err() {
            return false;
        }
        let Some(target) = self.target.clone() else {
            return false;
        };
        let Ok(format) = self.ensure_format(request.font, target_dpi(&target)) else {
            return false;
        };
        let text_bounds = text_area(request.bounds, request.text_bounds);
        let bound = windows::Win32::Foundation::RECT {
            left: request.bounds.left,
            top: request.bounds.top,
            right: request.bounds.right,
            bottom: request.bounds.bottom,
        };
        if unsafe { target.BindDC(HDC(request.hdc), &bound) }.is_err() {
            return false;
        }
        unsafe { target.BeginDraw() };
        let drew = draw_cell(
            &target,
            &format,
            request.bounds,
            text_bounds,
            request.text,
            request.colors,
        );
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

    fn ensure_format(
        &mut self,
        font: *mut core::ffi::c_void,
        dpi: f32,
    ) -> Result<IDWriteTextFormat, ()> {
        let Some(key) = font_key(font, dpi) else {
            return Err(());
        };
        if self.failed.as_ref() == Some(&key) {
            return Err(());
        }
        if let Some((cached, format)) = &self.format {
            if cached == &key {
                return Ok(format.clone());
            }
        }
        match text_format(&self.write, font, &key) {
            Ok(format) => {
                self.failed = None;
                self.format = Some((key, format.clone()));
                Ok(format)
            }
            Err(()) => {
                self.failed = Some(key);
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
    colors: RowColors,
) -> bool {
    // Inactive is the button face. That is the list view's unfocused selection
    // (`COLOR_3DFACE` is the same index as `COLOR_BTNFACE`).
    let (background, foreground) = match colors {
        RowColors::Highlight => (COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT),
        RowColors::Inactive => (COLOR_BTNFACE, COLOR_BTNTEXT),
        RowColors::Window => (COLOR_WINDOW, COLOR_WINDOWTEXT),
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

fn font_key(font: *mut core::ffi::c_void, dpi: f32) -> Option<FontKey> {
    if dpi <= 0.0 {
        return None;
    }
    let face = read_logfont(font)?;
    if face.lfFaceName[0] == 0 {
        return None;
    }
    let mut name = face.lfFaceName;
    name[31] = 0;
    let weight = if face.lfWeight <= 0 {
        400
    } else {
        face.lfWeight
    };
    Some(FontKey {
        face: name,
        height: face.lfHeight,
        weight,
        italic: u8::from(face.lfItalic != 0),
        dpi_bits: dpi.to_bits(),
    })
}

fn read_logfont(font: *mut core::ffi::c_void) -> Option<LOGFONTW> {
    if font.is_null() {
        return None;
    }
    let mut face: LOGFONTW = unsafe { mem::zeroed() };
    let wrote = unsafe {
        GetObjectW(
            font,
            mem::size_of::<LOGFONTW>() as i32,
            &mut face as *mut LOGFONTW as *mut core::ffi::c_void,
        )
    };
    if wrote < mem::size_of::<LOGFONTW>() as i32 {
        None
    } else {
        Some(face)
    }
}

/// Positive `lfHeight` is the cell height. The em size is that height minus
/// the font's internal leading.
fn measured_cell_em(font: *mut core::ffi::c_void) -> Option<i32> {
    let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let previous = unsafe { SelectObject(dc, font) };
    let mut metrics: TEXTMETRICW = unsafe { mem::zeroed() };
    let ok = unsafe { GetTextMetricsW(dc, &mut metrics) } != 0;
    if !previous.is_null() {
        unsafe {
            SelectObject(dc, previous);
        }
    }
    unsafe {
        DeleteDC(dc);
    }
    if !ok {
        return None;
    }
    let em = metrics.tmHeight.saturating_sub(metrics.tmInternalLeading);
    (em > 0).then_some(em)
}

fn text_format(
    write: &IDWriteFactory,
    font: *mut core::ffi::c_void,
    key: &FontKey,
) -> Result<IDWriteTextFormat, ()> {
    let em = negative_em(key.height)
        .or_else(|| measured_cell_em(font))
        .or_else(|| (key.height > 0).then_some(key.height))
        .ok_or(())?;
    let dpi = f32::from_bits(key.dpi_bits);
    let size = dip_size(em, dpi).ok_or(())?;
    let style = if key.italic != 0 {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    let format = unsafe {
        write.CreateTextFormat(
            PCWSTR(key.face.as_ptr()),
            None::<&IDWriteFontCollection>,
            DWRITE_FONT_WEIGHT(key.weight),
            style,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )
    }
    .map_err(|_| ())?;
    let _ = unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING) };
    let _ = unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER) };
    let _ = unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP) };
    Ok(format)
}

fn target_dpi(target: &ID2D1DCRenderTarget) -> f32 {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    unsafe { target.GetDpi(&mut x, &mut y) };
    if y > 0.0 {
        y
    } else {
        96.0
    }
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
