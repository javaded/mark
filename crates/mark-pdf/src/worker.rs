//! The single PDFium worker thread (plan.md §6.4).
//!
//! ```text
//! UI thread
//!    │  PdfWorker::load / render_page / close   (awaitable oneshot replies)
//!    ▼
//! std::sync::mpsc
//!    ▼
//! "mark-pdfium" thread — owns the Pdfium instance and every open
//! document, processes requests sequentially
//!    ▼
//! futures-channel oneshot — result delivered to whichever executor awaits
//! ```
//!
//! The worker thread ends when the last `PdfWorker` handle is dropped; the
//! application keeps one for its whole lifetime.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use futures_channel::oneshot;
use mark_core::{Document, PageGeometry};

use crate::bind;
use crate::document::load_pdf;
use crate::error::{LoadPdfError, RenderPageError};
use crate::render::{self, RenderedPage};

/// Opaque handle to a document opened on the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PdfDocumentHandle(u64);

impl PdfDocumentHandle {
    /// Test seam: constructs a handle from a raw id. Handles are normally
    /// issued only by [`PdfWorker::load`].
    #[doc(hidden)]
    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

/// A successfully loaded PDF: the worker-side handle, the domain document
/// (display page sizes + rotation), and the per-page user-space geometry.
#[derive(Debug)]
pub struct LoadedPdf {
    pub handle: PdfDocumentHandle,
    pub document: Document,
    pub pages: Vec<PageGeometry>,
}

enum Request {
    Load {
        path: PathBuf,
        reply: oneshot::Sender<Result<LoadedPdf, LoadPdfError>>,
    },
    RenderPage {
        document: PdfDocumentHandle,
        page_index: u32,
        target_width_px: u32,
        reply: oneshot::Sender<Result<RenderedPage, RenderPageError>>,
    },
    Close {
        document: PdfDocumentHandle,
    },
}

/// Handle to the PDFium worker thread. Cheap to clone via `Arc`.
pub struct PdfWorker {
    requests: mpsc::Sender<Request>,
}

impl PdfWorker {
    /// Spawns the dedicated PDFium worker thread.
    pub fn spawn() -> Self {
        let (requests, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("mark-pdfium".to_owned())
            .spawn(move || run(inbox))
            .expect("failed to spawn PDFium worker thread");
        Self { requests }
    }

    /// Loads a PDF on the worker thread. The returned receiver resolves to
    /// the document handle, domain document, and page geometry; it errors
    /// if the worker died mid-request.
    pub fn load(&self, path: PathBuf) -> oneshot::Receiver<Result<LoadedPdf, LoadPdfError>> {
        let (reply, received) = oneshot::channel();
        // A send error simply drops `reply`, which surfaces to the caller
        // as a RecvError — mapped to WorkerUnavailable by callers.
        let _ = self.requests.send(Request::Load { path, reply });
        received
    }

    /// Renders `page_index` of an open document at roughly `target_width_px`
    /// pixels wide (aspect preserved, intrinsic rotation applied).
    pub fn render_page(
        &self,
        document: PdfDocumentHandle,
        page_index: u32,
        target_width_px: u32,
    ) -> oneshot::Receiver<Result<RenderedPage, RenderPageError>> {
        let (reply, received) = oneshot::channel();
        let _ = self.requests.send(Request::RenderPage {
            document,
            page_index,
            target_width_px,
            reply,
        });
        received
    }

    /// Closes an open document, releasing it on the worker thread.
    pub fn close(&self, document: PdfDocumentHandle) {
        let _ = self.requests.send(Request::Close { document });
    }
}

fn run(inbox: mpsc::Receiver<Request>) {
    let Some(pdfium) = bind::try_bind() else {
        // No runtime: answer every request with RuntimeUnavailable so
        // callers get a real error instead of a hang (plan.md §9.4).
        for request in inbox {
            match request {
                Request::Load { reply, .. } => {
                    let _ = reply.send(Err(LoadPdfError::RuntimeUnavailable));
                }
                Request::RenderPage { reply, .. } => {
                    let _ = reply.send(Err(RenderPageError::WorkerUnavailable));
                }
                Request::Close { .. } => {}
            }
        }
        return;
    };

    let mut documents: HashMap<PdfDocumentHandle, pdfium_render::prelude::PdfDocument> =
        HashMap::new();
    let mut next_handle = 0_u64;

    for request in inbox {
        match request {
            Request::Load { path, reply } => {
                let _ = reply.send(load(&pdfium, path, &mut documents, &mut next_handle));
            }
            Request::RenderPage {
                document,
                page_index,
                target_width_px,
                reply,
            } => {
                let _ = reply.send(render(&documents, document, page_index, target_width_px));
            }
            Request::Close { document } => {
                documents.remove(&document);
            }
        }
    }
}

fn load<'a>(
    pdfium: &'a pdfium_render::prelude::Pdfium,
    path: PathBuf,
    documents: &mut HashMap<PdfDocumentHandle, pdfium_render::prelude::PdfDocument<'a>>,
    next_handle: &mut u64,
) -> Result<LoadedPdf, LoadPdfError> {
    let (pdf, document, pages) = load_pdf(pdfium, &path)?;
    let handle = PdfDocumentHandle(*next_handle);
    *next_handle += 1;
    documents.insert(handle, pdf);
    Ok(LoadedPdf {
        handle,
        document,
        pages,
    })
}

fn render(
    documents: &HashMap<PdfDocumentHandle, pdfium_render::prelude::PdfDocument<'_>>,
    document: PdfDocumentHandle,
    page_index: u32,
    target_width_px: u32,
) -> Result<RenderedPage, RenderPageError> {
    let pdf = documents
        .get(&document)
        .ok_or(RenderPageError::UnknownDocument)?;
    let page = pdf
        .pages()
        .get(page_index as i32)
        .map_err(RenderPageError::Render)?;
    render::render_page(&page, target_width_px)
}
