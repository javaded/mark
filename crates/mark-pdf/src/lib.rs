//! PDF document engine for Mark.
//!
//! Layer C of the architecture (plan.md §3): the only crate that talks to
//! PDFium. All PDFium access — loading, geometry, rendering, and (from
//! Phase 9) export — runs on a single dedicated worker thread reached
//! through [`PdfWorker`] (plan.md §6.4). No pdfium-render type crosses this
//! crate's public API.
//!
//! Coordinates: PDF user space (bottom-left origin, `/Rotate` and crop box
//! as-is) is described by [`mark_core::PageGeometry`]; this engine extracts
//! it and converts everything else to Mark's top-left display space via
//! [`mark_core::PageCoordinateMapper`].

// Runtime binding is public for the examples and tests; the application
// never touches it — it reaches PDFium only through `PdfWorker`.
pub mod bind;
mod document;
mod error;
pub mod export;
mod geometry;
mod render;
mod worker;
pub use error::{ExportPdfError, LoadPdfError, RenderPageError};
pub use export::{ExportProgress, ImageOverlay, PdfExport};
pub use render::RenderedPage;
pub use worker::{LoadedPdf, PdfDocumentHandle, PdfWorker};
