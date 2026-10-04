//! Error types for the PDF engine, mapped to friendly copy by the UI
//! (plan.md §9.4, §17). Technical detail stays in the `Debug` shape for
//! logs only.

#[derive(Debug, thiserror::Error)]
pub enum LoadPdfError {
    #[error("the PDF could not be opened")]
    Open(#[from] pdfium_render::prelude::PdfiumError),
    #[error("the PDFium runtime could not be loaded")]
    RuntimeUnavailable,
    #[error("the PDF engine is unavailable")]
    WorkerUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderPageError {
    #[error("the page could not be rendered")]
    Render(#[from] pdfium_render::prelude::PdfiumError),
    #[error("no such open document")]
    UnknownDocument,
    #[error("the PDF engine is unavailable")]
    WorkerUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportPdfError {
    #[error("the PDF could not be exported")]
    Pdfium(#[from] pdfium_render::prelude::PdfiumError),
    #[error("an overlay image could not be decoded")]
    Image(#[from] image::ImageError),
    #[error("the PDFium runtime could not be loaded")]
    RuntimeUnavailable,
    #[error("the PDF engine is unavailable")]
    WorkerUnavailable,
}
