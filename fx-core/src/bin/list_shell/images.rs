#![cfg_attr(not(windows), allow(dead_code))]
//! Which names can have a thumbnail, and the Windows decode that produces one.
//!
//! The extension check does not open a file. The decode runs on a thumbnail
//! worker, not the UI thread. A frame larger than 16384 pixels on a side is
//! refused. That guard is not a performance budget.

#[cfg(windows)]
use fx_core::thumbs::Rgba;

/// Longest edge of a decoded thumbnail, in pixels. Not a performance budget.
const THUMB_EDGE: u32 = 64;
/// Frames larger than this on either side are not decoded.
const MAX_SOURCE_EDGE: u32 = 16_384;

const IMAGE_EXTENSIONS: &[&str] = &[
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

pub fn wants_thumbnail(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    if name.starts_with('.') && name.matches('.').count() == 1 {
        return false;
    }
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if extension.is_empty() {
        return false;
    }
    let extension = extension.to_ascii_lowercase();
    IMAGE_EXTENSIONS.iter().any(|known| *known == extension)
}

/// Fit `width` and `height` inside `edge` without growing a smaller image.
pub fn fit_edge(width: u32, height: u32, edge: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || edge == 0 || width > MAX_SOURCE_EDGE || height > MAX_SOURCE_EDGE
    {
        return None;
    }
    if width <= edge && height <= edge {
        return Some((width, height));
    }
    let width = u64::from(width);
    let height = u64::from(height);
    let edge = u64::from(edge);
    if width >= height {
        let fitted = (height * edge / width).max(1);
        Some((edge as u32, fitted as u32))
    } else {
        let fitted = (width * edge / height).max(1);
        Some((fitted as u32, edge as u32))
    }
}

/// Decode one image to RGBA8. Image extensions only. Does not run on the UI thread
/// when called from [`fx_core::thumbs::ThumbCache::with_decoder`].
#[cfg(windows)]
pub fn decode_image(path: &str) -> Result<Rgba, ()> {
    if !wants_thumbnail(path) {
        return Err(());
    }
    decode_scaled(path)
}

#[cfg(windows)]
fn decode_scaled(path: &str) -> Result<Rgba, ()> {
    use std::cell::{Cell, RefCell};

    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
        WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };

    thread_local! {
        static COM_READY: Cell<bool> = const { Cell::new(false) };
        static FACTORY: RefCell<Option<IWICImagingFactory>> = const { RefCell::new(None) };
    }

    COM_READY.with(|ready| {
        if ready.get() {
            return Ok(());
        }
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // `RPC_E_CHANGED_MODE`: this thread already has an apartment.
        if hr.is_err() && hr.0 != 0x8001_0106_u32 as i32 {
            return Err(());
        }
        ready.set(true);
        Ok(())
    })?;

    let factory = FACTORY.with(|slot| {
        if let Some(factory) = slot.borrow().as_ref() {
            return Ok(factory.clone());
        }
        let created: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| ())?;
        *slot.borrow_mut() = Some(created.clone());
        Ok(created)
    })?;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let decoder = unsafe {
        factory.CreateDecoderFromFilename(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            windows::Win32::Foundation::GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }
    .map_err(|_| ())?;
    let frame = unsafe { decoder.GetFrame(0) }.map_err(|_| ())?;
    let mut source_width = 0u32;
    let mut source_height = 0u32;
    unsafe { frame.GetSize(&mut source_width, &mut source_height) }.map_err(|_| ())?;
    let Some((width, height)) = fit_edge(source_width, source_height, THUMB_EDGE) else {
        return Err(());
    };
    let converter = unsafe { factory.CreateFormatConverter() }.map_err(|_| ())?;
    unsafe {
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None::<&windows::Win32::Graphics::Imaging::IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }
    .map_err(|_| ())?;
    let scaler = unsafe { factory.CreateBitmapScaler() }.map_err(|_| ())?;
    unsafe { scaler.Initialize(&converter, width, height, WICBitmapInterpolationModeFant) }
        .map_err(|_| ())?;
    let stride = width.checked_mul(4).ok_or(())?;
    let len = usize::try_from(stride.checked_mul(height).ok_or(())?).map_err(|_| ())?;
    let mut pixels = vec![0u8; len];
    unsafe { scaler.CopyPixels(std::ptr::null(), stride, &mut pixels) }.map_err(|_| ())?;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_image_extensions_are_thumbnails() {
        assert!(wants_thumbnail(r"C:\photos\cat.PNG"));
        assert!(wants_thumbnail(r"C:\photos\cat.jpeg"));
        assert!(wants_thumbnail("/tmp/a.webp"));
        assert!(!wants_thumbnail(r"C:\photos\notes.txt"));
        assert!(!wants_thumbnail(r"C:\photos\archive.tar.gz"));
        assert!(!wants_thumbnail(r"C:\photos\folder"));
        assert!(!wants_thumbnail(".png"));
    }

    #[test]
    fn fit_edge_shrinks_the_long_side_and_refuses_a_huge_frame() {
        assert_eq!(fit_edge(32, 16, 64), Some((32, 16)));
        assert_eq!(fit_edge(128, 64, 64), Some((64, 32)));
        assert_eq!(fit_edge(50, 100, 64), Some((32, 64)));
        assert_eq!(fit_edge(0, 10, 64), None);
        assert_eq!(fit_edge(20_000, 10, 64), None);
    }
}
