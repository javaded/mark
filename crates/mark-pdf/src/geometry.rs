//! Extracting [`PageGeometry`] from PDFium pages.

use mark_core::{PageGeometry, PageRotation, Vec2};
use pdfium_render::prelude::{PdfPage, PdfPageRenderRotation};

/// Reads a page's effective box (crop box, falling back to media box) and
/// intrinsic rotation as a [`PageGeometry`].
///
/// The geometry is in PDF user space: `origin` is the box's bottom-left
/// corner, `size` is unrotated. The displayed page size is
/// `geometry.display_size()`.
pub(crate) fn page_geometry(
    page: &PdfPage,
) -> Result<PageGeometry, pdfium_render::prelude::PdfiumError> {
    let rotation = page_rotation(page.rotation()?);
    // FPDFPage_GetCropBox already falls back to the media box when no crop
    // box is defined (PDF spec); the explicit fallback covers any pdfium
    // build that reports failure instead.
    let bounds = page
        .boundaries()
        .crop()
        .or_else(|_| page.boundaries().media())?
        .bounds;
    Ok(PageGeometry::new(
        Vec2::new(bounds.left().value, bounds.bottom().value),
        Vec2::new(bounds.width().value, bounds.height().value),
        rotation,
    ))
}

fn page_rotation(rotation: PdfPageRenderRotation) -> PageRotation {
    PageRotation::from_degrees(rotation.as_degrees() as u16)
}
