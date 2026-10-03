//! Loading PDFs: PDFium document → Mark domain document.

use std::path::Path;

use mark_core::{Document, DocumentSource, Page, PageGeometry};
use pdfium_render::prelude::Pdfium;

use crate::error::LoadPdfError;
use crate::geometry::page_geometry;

/// Opens the PDF at `path` on the given `pdfium` instance, returning the
/// live PDFium document, per-page geometry, and a domain [`Document`]
/// whose pages carry display sizes (post-rotation) and rotation metadata
/// (plan.md §8.1).
pub(crate) fn load_pdf<'a>(
    pdfium: &'a Pdfium,
    path: &Path,
) -> Result<
    (
        pdfium_render::prelude::PdfDocument<'a>,
        Document,
        Vec<PageGeometry>,
    ),
    LoadPdfError,
> {
    let pdf = pdfium.load_pdf_from_file(path, None)?;

    let mut geometries = Vec::with_capacity(pdf.pages().len() as usize);
    let mut pages = Vec::with_capacity(pdf.pages().len() as usize);
    for page in pdf.pages().iter() {
        let geometry = page_geometry(&page)?;
        let display = geometry.display_size();
        pages.push(Page::with_rotation(display.x, display.y, geometry.rotation));
        geometries.push(geometry);
    }

    let document = Document::new(
        DocumentSource::Pdf {
            path: path.to_path_buf(),
        },
        pages,
    );
    Ok((pdf, document, geometries))
}
