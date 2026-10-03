//! Document domain model for Mark.
//!
//! Layer B of the architecture (plan.md §3): pages, placed objects, object
//! transformations, selection, undo/redo, and document/session state.
//!
//! This crate must never depend on GPUI or PDFium.
//!
//! Coordinate convention: all positions and sizes are in logical page units,
//! where 1 inch = 72 points (plan.md §8). The origin is the page's top-left
//! corner; conversions to PDF's bottom-left origin happen only in the PDF
//! engine at export time.

mod commands;
mod coordinates;
mod ids;
mod model;
mod session;
mod transform;

pub use commands::Command;
pub use commands::{
    AddObject, DeleteObject, DuplicateObject, DuplicateToPage, MoveObject, MoveObjectToPage,
    ResizeObject, RotateObject, SetOpacity,
};
pub use coordinates::{PageCoordinateMapper, PageGeometry};
pub use ids::{AssetId, ObjectId, PageId};
pub use model::{
    Asset, AssetKind, Document, DocumentObject, DocumentSource, ImageObject, ObjectKind, Page,
    PageRotation, Rect, Vec2,
};
pub use session::{DocumentSession, UndoManager};
pub use transform::ViewTransform;
