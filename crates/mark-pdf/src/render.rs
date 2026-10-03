//! Rendering PDF pages to RGBA bitmaps (plan.md §11).
//!
//! Pages render at the display size implied by `target_width_px` — the
//! page's intrinsic `/Rotate` is applied by PDFium, so the bitmap shows the
//! same orientation the viewer displays.

use image::RgbaImage;
use pdfium_render::prelude::{PdfPage, PdfRenderConfig};

use crate::error::RenderPageError;
use crate::geometry::page_geometry;

/// A rendered page: RGBA pixels plus the render scale (pixels per point).
#[derive(Debug)]
pub struct RenderedPage {
    pub rgba: RgbaImage,
    pub scale: f32,
}

pub(crate) fn render_page(
    page: &PdfPage,
    target_width_px: u32,
) -> Result<RenderedPage, RenderPageError> {
    let config = PdfRenderConfig::new().set_target_width(target_width_px as i32);
    let bitmap = page.render_with_config(&config)?;
    let rgba = bitmap.as_image()?.to_rgba8();

    let display_width = page_geometry(page)?.display_size().x;
    let scale = if display_width > 0.0 {
        rgba.width() as f32 / display_width
    } else {
        1.0
    };
    Ok(RenderedPage { rgba, scale })
}
