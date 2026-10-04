//! Export composition and destination naming (plan.md §16).
//!
//! Image documents export as PNG: the page's base bitmap with every
//! placed object composited on top, alpha respected (plan.md §9.3). PDF
//! export lives in `mark-pdf` (PDFium writes real image page objects,
//! §16.1); this module owns everything format-independent.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use image::imageops::FilterType;
use mark_core::Rect;

/// The default export destination: `<original-stem>-signed.<ext>` next to
/// the source (plan.md §16) — never the original path.
pub fn signed_destination(source: &Path, extension: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document");
    source.with_file_name(format!("{stem}-signed.{extension}"))
}

/// One object composited onto the exported page image.
pub struct PngOverlay<'a> {
    /// Placement in page points. Image documents are 1 px = 1 pt
    /// (plan.md §8), so point coordinates map 1:1 to pixels.
    pub rect: Rect,
    /// The asset's decoded RGBA bitmap.
    pub image: &'a RgbaImage,
    /// 0.0..=1.0, applied to the overlay's alpha.
    pub opacity: f32,
}

/// Composites `overlays` over `base` in z-order (last drawn on top).
///
/// Mirrors what the canvas shows: each bitmap scales to its rect with
/// alpha multiplied by the object's opacity. Objects' `rotation` is not
/// applied — the editor canvas does not render it either (Phase 8
/// decision); rotation rendering and export arrive together.
pub fn compose_png_page(base: &RgbaImage, overlays: &[PngOverlay<'_>]) -> RgbaImage {
    let mut page = base.clone();
    for overlay in overlays {
        let width = overlay.rect.width.round().max(1.) as u32;
        let height = overlay.rect.height.round().max(1.) as u32;
        let mut scaled =
            image::imageops::resize(overlay.image, width, height, FilterType::Lanczos3);
        apply_opacity(&mut scaled, overlay.opacity);
        let x = overlay.rect.x.round() as i64;
        let y = overlay.rect.y.round() as i64;
        image::imageops::overlay(&mut page, &scaled, x, y);
    }
    page
}

/// Multiplies every pixel's alpha by `opacity` (clamped to 0..=1).
fn apply_opacity(image: &mut RgbaImage, opacity: f32) {
    if opacity >= 1.0 {
        return;
    }
    let factor = opacity.clamp(0., 1.);
    for pixel in image.pixels_mut() {
        pixel.0[3] = (f32::from(pixel.0[3]) * factor).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(rgba))
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn signed_destination_appends_suffix_and_swaps_extension() {
        let dest = signed_destination(Path::new("/docs/contract.pdf"), "pdf");
        assert_eq!(dest, PathBuf::from("/docs/contract-signed.pdf"));
        // An image source exporting to PNG swaps the extension.
        let dest = signed_destination(Path::new("/pics/scan.jpg"), "png");
        assert_eq!(dest, PathBuf::from("/pics/scan-signed.png"));
        // Never the original path, even for odd names.
        let dest = signed_destination(Path::new("weird"), "png");
        assert_eq!(dest, PathBuf::from("weird-signed.png"));
    }

    #[test]
    fn composition_places_overlay_pixels_and_keeps_base_elsewhere() {
        let base = solid(100, 80, [255, 255, 255, 255]);
        let stamp = solid(40, 30, [200, 30, 30, 255]);
        let overlays = [PngOverlay {
            rect: rect(10., 20., 40., 30.),
            image: &stamp,
            opacity: 1.0,
        }];
        let out = compose_png_page(&base, &overlays);
        assert_eq!(out.dimensions(), (100, 80));
        // Center of the placement: the stamp.
        assert_eq!(out.get_pixel(30, 35), &image::Rgba([200, 30, 30, 255]));
        // Just outside: the base.
        assert_eq!(out.get_pixel(5, 5), &image::Rgba([255, 255, 255, 255]));
        assert_eq!(out.get_pixel(60, 55), &image::Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn composition_resizes_overlays_to_their_rect() {
        let base = solid(200, 200, [255, 255, 255, 255]);
        // 10×10 asset scaled up to 100×100.
        let stamp = solid(10, 10, [0, 0, 255, 255]);
        let overlays = [PngOverlay {
            rect: rect(50., 50., 100., 100.),
            image: &stamp,
            opacity: 1.0,
        }];
        let out = compose_png_page(&base, &overlays);
        assert_eq!(out.get_pixel(100, 100), &image::Rgba([0, 0, 255, 255]));
        assert_eq!(out.get_pixel(149, 149), &image::Rgba([0, 0, 255, 255]));
        assert_eq!(out.get_pixel(45, 100), &image::Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn opacity_blends_transparent_stamps_with_the_base() {
        let base = solid(10, 10, [100, 100, 100, 255]);
        let stamp = solid(10, 10, [0, 0, 255, 255]);
        let overlays = [PngOverlay {
            rect: rect(0., 0., 10., 10.),
            image: &stamp,
            opacity: 0.5,
        }];
        let out = compose_png_page(&base, &overlays);
        // 50% blue over mid gray: channel mix.
        let pixel = out.get_pixel(5, 5);
        assert!(pixel.0[3] >= 253, "fully opaque base shows through");
        assert!((pixel.0[2] as i32 - 178).abs() <= 1, "blended blue");
        assert!((pixel.0[0] as i32 - 50).abs() <= 1, "blended gray");
    }
}
