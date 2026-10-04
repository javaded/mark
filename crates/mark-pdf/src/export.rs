//! Exporting signed PDFs (plan.md §16.1): the original PDF plus overlay
//! objects, written by PDFium as real image page objects — never a
//! screenshot of the canvas, never selection UI.
//!
//! Every coordinate conversion goes through
//! [`mark_core::PageCoordinateMapper`]: overlays arrive in display space
//! (top-left origin, post-rotation) and are placed onto the user-space quad
//! the viewer shows them at, uniformly for every `/Rotate` and crop box.

use std::path::PathBuf;

use mark_core::{PageCoordinateMapper, Rect, Vec2};
use pdfium_render::prelude::{PdfMatrix, PdfPageObjectsCommon as _, PdfPoints, Pdfium};

use crate::error::ExportPdfError;
use crate::geometry::page_geometry;

/// One overlay image placed on a page: its rect in Mark display
/// coordinates (points, top-left origin, post-rotation) and the encoded
/// PNG bytes (the library's normalized asset).
#[derive(Debug, Clone)]
pub struct ImageOverlay {
    pub rect: Rect,
    pub png: Vec<u8>,
}

/// A full export job: source → destination with per-page overlays.
#[derive(Debug)]
pub struct PdfExport {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// Overlays per page, in page order; empty pages export untouched.
    pub pages: Vec<Vec<ImageOverlay>>,
}

/// Per-page progress reported while an export runs (plan.md §11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    pub done: u32,
    pub total: u32,
}

/// The matrix placing an image's unit square onto the user-space quad
/// `rect` occupies on screen: the image's bottom-left corner maps to the
/// display rect's bottom-left (through the page mapper), with the bottom
/// and left edges as the matrix's basis vectors.
///
/// Rotations by quarter turns keep the quad axis-aligned, so this handles
/// every `/Rotate` and crop box offset uniformly; the determinant stays
/// `width × height > 0`, so the image is never mirrored.
pub(crate) fn placement_matrix(mapper: &PageCoordinateMapper, rect: Rect) -> PdfMatrix {
    let top_left = mapper.display_to_user(Vec2::new(rect.x, rect.y));
    let bottom_left = mapper.display_to_user(Vec2::new(rect.x, rect.y + rect.height));
    let bottom_right = mapper.display_to_user(Vec2::new(rect.x + rect.width, rect.y + rect.height));
    PdfMatrix::new(
        bottom_right.x - bottom_left.x,
        bottom_right.y - bottom_left.y,
        top_left.x - bottom_left.x,
        top_left.y - bottom_left.y,
        bottom_left.x,
        bottom_left.y,
    )
}

/// Runs the export (on the PDFium worker thread, plan.md §6.4): loads the
/// source fresh — the document open in the editor is never mutated —
/// places every overlay as a real image page object, and saves to the
/// destination. `on_page_done` reports `(pages_done, pages_total)` after
/// each page that received overlays.
pub(crate) fn export_pdf(
    pdfium: &Pdfium,
    export: &PdfExport,
    mut on_page_done: impl FnMut(u32, u32),
) -> Result<(), ExportPdfError> {
    let pdf = pdfium.load_pdf_from_file(&export.source, None)?;
    let total = export.pages.len() as u32;
    for (index, overlays) in export.pages.iter().enumerate() {
        if overlays.is_empty() {
            continue;
        }
        let pages = pdf.pages();
        let mut page = pages.get(index as i32)?;
        let mapper = page_geometry(&page)?.mapper();
        let objects = page.objects_mut();
        for overlay in overlays {
            let image = image::load_from_memory(&overlay.png)?;
            // Created 1×1 at the origin (identity matrix), then given the
            // full placement matrix: composing over the identity leaves
            // exactly the placement we computed.
            let mut object = objects.create_image_object(
                PdfPoints::ZERO,
                PdfPoints::ZERO,
                &image,
                None,
                None,
            )?;
            object.apply_matrix(placement_matrix(&mapper, overlay.rect))?;
        }
        on_page_done(index as u32 + 1, total);
    }
    pdf.save_to_file(&export.destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mark_core::{PageGeometry, PageRotation};

    fn mapper(rotation: PageRotation) -> PageCoordinateMapper {
        PageGeometry::new(Vec2::new(0., 0.), Vec2::new(200., 100.), rotation).mapper()
    }

    fn rect() -> Rect {
        Rect {
            x: 20.,
            y: 30.,
            width: 40.,
            height: 10.,
        }
    }

    /// Where the matrix sends the unit square's corners.
    fn corners(matrix: PdfMatrix) -> [(f32, f32); 4] {
        let point = |u: f32, v: f32| {
            (
                matrix.a() * u + matrix.c() * v + matrix.e(),
                matrix.b() * u + matrix.d() * v + matrix.f(),
            )
        };
        [point(0., 0.), point(1., 0.), point(1., 1.), point(0., 1.)]
    }

    #[test]
    fn matrix_places_rect_bottom_left_for_unrotated_pages() {
        // Display (20,30,40×10): user space is (20, 100-40)–(60, 100-30)
        // = bottom-left (20, 60), top-right (60, 70).
        let m = placement_matrix(&mapper(PageRotation::None), rect());
        assert_eq!(corners(m)[0], (20., 60.)); // image bottom-left
        assert_eq!(corners(m)[1], (60., 60.)); // image bottom-right
        assert_eq!(corners(m)[3], (20., 70.)); // image top-left
        assert!(m.a() > 0. && m.d() > 0., "upright, not mirrored");
    }

    #[test]
    fn matrix_rotates_with_rotate_90_pages() {
        // /Rotate 90: display_to_user(d) = (d.y, d.x). Image bottom-left
        // (20, 40) → user (40, 20); the image is rotated with the page so
        // it appears upright after the viewer applies /Rotate.
        let m = placement_matrix(&mapper(PageRotation::Degrees90), rect());
        assert_eq!(corners(m)[0], (40., 20.));
        assert_eq!(corners(m)[1], (40., 60.)); // bottom edge runs up +y
        assert_eq!(corners(m)[3], (30., 20.)); // left edge runs along -x
        let determinant = m.a() * m.d() - m.b() * m.c();
        assert!(determinant > 0., "rotation, not a mirror");
    }

    #[test]
    fn matrix_keeps_crop_box_offsets() {
        let geometry = PageGeometry::new(
            Vec2::new(61.2, 79.2),
            Vec2::new(200., 100.),
            PageRotation::None,
        );
        let m = placement_matrix(&geometry.mapper(), rect());
        // The crop origin shifts every user coordinate.
        assert_eq!(corners(m)[0], (61.2 + 20., 79.2 + 60.));
    }
}
