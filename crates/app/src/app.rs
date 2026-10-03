//! Main application view: header bar, workspace states, open flow.
//!
//! Phase 3 opened images; Phase 4 adds PDFs — loads and renders run on the
//! PDFium worker thread (plan.md §6.4), never on the UI thread. Thumbnails,
//! page navigation, and the asset panel arrive in later phases.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{
    ClickEvent, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, rems,
};
use gpui_omarchy::{ActiveTheme, ButtonVariant, IconName, Theme, button, focus_scope, icon};
use mark_core::DocumentSession;
use mark_pdf::{PdfDocumentHandle, PdfWorker};

use crate::OpenDocument;
use crate::canvas;

/// Everything needed to display an opened document.
///
/// `page_image` is the current page's render — present immediately for
/// image documents, filled in asynchronously for PDFs.
pub struct OpenedDocument {
    session: DocumentSession,
    page_image: Option<Arc<gpui_kit::RenderImage>>,
    /// Set for PDF documents: the worker-side handle for future render
    /// requests (Phase 5 navigation/zoom) and closing.
    pdf: Option<PdfDocumentHandle>,
}

impl OpenedDocument {
    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    pub fn page_image(&self) -> Option<&Arc<gpui_kit::RenderImage>> {
        self.page_image.as_ref()
    }
}

enum OpenState {
    Empty,
    Opening { name: SharedString },
    Failed { name: SharedString },
    Opened(OpenedDocument),
}

pub struct MarkApp {
    open: OpenState,
    pdf: Arc<PdfWorker>,
}

impl MarkApp {
    pub fn new(pdf: Arc<PdfWorker>) -> Self {
        Self {
            open: OpenState::Empty,
            pdf,
        }
    }

    /// Opens the native file dialog and loads whatever the user picks.
    fn open_with_dialog(&mut self, cx: &mut Context<Self>) {
        let dialogs = platform::NativeFileDialogs;
        cx.spawn(async move |this, cx| {
            let Some(path) = platform::FilePicker::pick_open_document(&dialogs).await else {
                return;
            };
            this.update(cx, |app, cx| app.open_path(path, cx)).ok();
        })
        .detach();
    }

    /// Loads `path` as a document, images and PDFs alike, off the UI thread.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if is_pdf(&path) {
            self.open_pdf(path, cx);
        } else {
            self.open_image(path, cx);
        }
    }

    /// Releases the previous document's worker-side resources, if any.
    fn close_previous(&mut self) {
        if let OpenState::Opened(opened) = &self.open
            && let Some(handle) = opened.pdf
        {
            self.pdf.close(handle);
        }
    }

    fn open_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name: SharedString = file_name(&path).into();
        self.open = OpenState::Opening { name: name.clone() };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { mark_image::ImageDocument::load(&path) })
                .await;
            this.update(cx, |app, cx| {
                app.open = match loaded {
                    Ok(image_document) => {
                        let (document, rgba) = image_document.into_parts();
                        OpenState::Opened(OpenedDocument {
                            session: DocumentSession::new(document),
                            page_image: Some(Arc::new(canvas::render_image(&rgba))),
                            pdf: None,
                        })
                    }
                    Err(_) => OpenState::Failed { name },
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Loads a PDF on the PDFium worker, then renders page 1 (plan.md §6.4).
    fn open_pdf(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name: SharedString = file_name(&path).into();
        self.close_previous();
        self.open = OpenState::Opening { name: name.clone() };
        cx.notify();

        let worker = self.pdf.clone();
        cx.spawn(async move |this, cx| {
            // Load: document metadata plus the worker-side handle. The app
            // only distinguishes success from failure here; friendly error
            // copy arrives with the error surfaces (Phase 10, plan.md §17).
            let loaded: Option<mark_pdf::LoadedPdf> =
                worker.load(path).await.ok().and_then(|result| result.ok());

            // None = load failure or worker death; either way, Failed.
            let rendered = match loaded {
                Some(loaded) => {
                    let handle = loaded.handle;
                    // Page-0 render target: 2× points, clamped to a
                    // sane band until zoom lands (Phase 5, plan.md §11).
                    let display_width = loaded
                        .pages
                        .first()
                        .map(|geometry| geometry.display_size().x)
                        .unwrap_or(612.0);
                    let target_width = (display_width * 2.0).round().clamp(800.0, 2400.0) as u32;

                    // Show the document immediately; the page render fills
                    // in when the worker replies.
                    this.update(cx, |app, cx| {
                        app.open = OpenState::Opened(OpenedDocument {
                            session: DocumentSession::new(loaded.document),
                            page_image: None,
                            pdf: Some(handle),
                        });
                        cx.notify();
                    })
                    .ok();

                    worker
                        .render_page(handle, 0, target_width)
                        .await
                        .ok()
                        .and_then(|result| result.ok())
                }
                None => None,
            };

            this.update(cx, |app, cx| {
                match rendered {
                    Some(rendered) => {
                        if let OpenState::Opened(opened_doc) = &mut app.open {
                            opened_doc.page_image =
                                Some(Arc::new(canvas::render_image(&rendered.rgba)));
                        }
                    }
                    None => app.open = OpenState::Failed { name },
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn header_title(&self) -> Option<&str> {
        match &self.open {
            OpenState::Opened(opened) => opened.session().source().path().file_name()?.to_str(),
            _ => None,
        }
    }

    fn open_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "⌘O"
        } else {
            "Ctrl+O"
        }
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
}

impl Render for MarkApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();

        focus_scope("mark")
            .on_action(cx.listener(|this, _: &OpenDocument, _window, cx| {
                this.open_with_dialog(cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.inset)
            .child(header(&theme, self.header_title()))
            .child(match &self.open {
                OpenState::Empty => empty_workspace(&theme, cx).into_any_element(),
                OpenState::Opening { name } => {
                    notice_workspace(&theme, IconName::FileText, format!("Opening {name}…"), None)
                        .into_any_element()
                }
                OpenState::Failed { name } => notice_workspace(
                    &theme,
                    IconName::FileX,
                    format!("Could not open {name}"),
                    Some("The file may be damaged or in an unsupported format."),
                )
                .into_any_element(),
                OpenState::Opened(opened) => match opened.page_image() {
                    Some(image) => canvas::page(&theme, image).into_any_element(),
                    None => notice_workspace(
                        &theme,
                        IconName::FileText,
                        "Rendering page…".to_owned(),
                        None,
                    )
                    .into_any_element(),
                },
            })
    }
}

fn header(theme: &Theme, title: Option<&str>) -> impl IntoElement {
    div()
        .id("mark-header")
        .flex()
        .items_center()
        .justify_between()
        .px(rems(1.))
        .h(rems(2.75))
        .bg(theme.background)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(rems(0.5))
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child("Mark"),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(title.unwrap_or("Sign anything.").to_owned()),
                ),
        )
        // Zoom controls and Export button arrive with the document canvas (Phase 5+).
        .child(div().id("mark-header-actions"))
}

fn empty_workspace(theme: &Theme, cx: &mut Context<MarkApp>) -> impl IntoElement {
    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(0.75))
                .child(
                    icon(IconName::FileText)
                        .size(rems(2.5))
                        .text_color(theme.secondary),
                )
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .text_color(theme.foreground)
                        .child("No document open"),
                )
                .child(
                    button("open-document", "Open…", ButtonVariant::Outline, cx).on_click(
                        cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.open_with_dialog(cx);
                        }),
                    ),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(format!("or press {}", MarkApp::open_hint())),
                ),
        )
}

fn notice_workspace(
    theme: &Theme,
    icon_name: IconName,
    title: String,
    detail: Option<&str>,
) -> impl IntoElement {
    let detail = detail.map(|detail| {
        div()
            .text_size(rems(0.75))
            .text_color(theme.secondary)
            .child(detail.to_owned())
    });

    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(0.75))
                .child(icon(icon_name).size(rems(2.5)).text_color(theme.secondary))
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .text_color(theme.foreground)
                        .child(title),
                )
                .children(detail),
        )
}
