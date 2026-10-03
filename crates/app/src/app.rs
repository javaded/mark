//! Main application view: header bar, workspace states, open flow, page
//! navigation, zoom, and the render-request loop.
//!
//! Phases 3–4 opened images and PDFs; Phase 5 adds the viewer: lazy
//! thumbnails (§11.2), keyboard + status-bar navigation, and zoom/pan/fit
//! with viewport-sized re-rendering (§11.1). All PDFium work stays on the
//! worker thread (§6.4); this view only decides *what* to request and
//! records replies.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{
    ClickEvent, Context, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div, rems,
};
use gpui_omarchy::{ActiveTheme, ButtonVariant, IconName, Theme, button, focus_scope, icon};
use mark_core::{DocumentSession, Vec2};
use mark_pdf::{PdfDocumentHandle, PdfWorker, RenderedPage};

use crate::canvas;
use crate::thumbnails;
use crate::viewer::ViewerState;
use crate::{
    FirstPage as FirstPageAction, LastPage as LastPageAction, NextPage as NextPageAction,
    OpenDocument, PreviousPage as PreviousPageAction, ZoomFit as ZoomFitAction,
    ZoomIn as ZoomInAction, ZoomOut as ZoomOutAction,
};

/// Everything needed to display an opened document.
///
/// `viewer` owns view state (current page, zoom/pan, render cache,
/// thumbnails); document content lives in the session (plan.md §7.1).
pub struct OpenedDocument {
    session: DocumentSession,
    /// Set for PDF documents: the worker-side handle for render requests
    /// and closing.
    pdf: Option<PdfDocumentHandle>,
    viewer: ViewerState,
}

impl OpenedDocument {
    pub fn session(&self) -> &DocumentSession {
        &self.session
    }
}

enum OpenState {
    Empty,
    Opening { name: SharedString },
    Failed { name: SharedString },
    Opened(Box<OpenedDocument>),
}

pub struct MarkApp {
    open: OpenState,
    pdf: Arc<PdfWorker>,
    /// Keyboard focus for the whole app: actions (navigation, zoom, open)
    /// dispatch through the focused node.
    focus_handle: FocusHandle,
}

impl MarkApp {
    pub fn new(pdf: Arc<PdfWorker>, cx: &mut Context<Self>) -> Self {
        Self {
            open: OpenState::Empty,
            pdf,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn viewer_ref(&self) -> &ViewerState {
        match &self.open {
            OpenState::Opened(opened) => &opened.viewer,
            _ => unreachable!("viewer access requires an open document"),
        }
    }

    pub(crate) fn viewer_mut(&mut self) -> &mut ViewerState {
        match &mut self.open {
            OpenState::Opened(opened) => &mut opened.viewer,
            _ => unreachable!("viewer access requires an open document"),
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
                        let page = document
                            .pages()
                            .first()
                            .map(|page| Vec2::new(page.width(), page.height()))
                            .unwrap_or_else(|| Vec2::new(612., 792.));
                        let mut viewer = ViewerState::new_image(page);
                        viewer.seed_page_image(
                            0,
                            rgba.width(),
                            Arc::new(canvas::render_image(&rgba)),
                        );
                        OpenState::Opened(Box::new(OpenedDocument {
                            session: DocumentSession::new(document),
                            pdf: None,
                            viewer,
                        }))
                    }
                    Err(_) => OpenState::Failed { name },
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Loads a PDF on the PDFium worker (plan.md §6.4); page renders and
    /// thumbnails follow from `refresh_view` once the viewport is measured.
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

            this.update(cx, |app, cx| {
                match loaded {
                    Some(loaded) => {
                        let handle = loaded.handle;
                        let page_sizes = loaded
                            .pages
                            .iter()
                            .map(|geometry| geometry.display_size())
                            .collect();
                        app.open = OpenState::Opened(Box::new(OpenedDocument {
                            session: DocumentSession::new(loaded.document),
                            pdf: Some(handle),
                            viewer: ViewerState::new_pdf(page_sizes),
                        }));
                        cx.notify();
                        // The viewport is still unknown; the canvas probe
                        // triggers the first render request after layout.
                        app.refresh_view(cx);
                    }
                    None => {
                        app.open = OpenState::Failed { name };
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    // ----- navigation and zoom ------------------------------------------------

    pub(crate) fn go_to_page(&mut self, page: u32, cx: &mut Context<Self>) {
        self.navigate_with(cx, |viewer| viewer.navigate_to(page))
    }

    fn handle_next_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::next_page);
    }

    fn handle_previous_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::previous_page);
    }

    fn handle_first_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::first_page);
    }

    fn handle_last_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::last_page);
    }

    fn navigate_with(
        &mut self,
        cx: &mut Context<Self>,
        navigate: impl FnOnce(&mut ViewerState) -> bool,
    ) {
        let mut changed = false;
        if let OpenState::Opened(opened) = &mut self.open {
            changed = navigate(&mut opened.viewer);
            if changed {
                opened
                    .viewer
                    .prioritize_thumbnail(opened.viewer.current_page());
                cx.notify();
            }
        }
        if changed {
            self.refresh_view(cx);
        }
    }

    fn handle_zoom_in(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_in);
    }

    fn handle_zoom_out(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_out);
    }

    fn handle_zoom_fit(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_fit);
    }

    fn zoom_with(&mut self, cx: &mut Context<Self>, zoom: impl FnOnce(&mut ViewerState)) {
        if let OpenState::Opened(opened) = &mut self.open {
            zoom(&mut opened.viewer);
            cx.notify();
        }
        self.refresh_view(cx);
    }

    // ----- render request loop (plan.md §11) -------------------------------------

    /// Issues the requests the current view needs: the current page's
    /// bitmap at viewport resolution, then queued thumbnails.
    pub(crate) fn refresh_view(&mut self, cx: &mut Context<Self>) {
        let wanted = match &self.open {
            OpenState::Opened(opened) => opened.pdf.zip(opened.viewer.wanted_render()),
            _ => None,
        };
        if let Some((handle, (page, width))) = wanted {
            if let OpenState::Opened(opened) = &mut self.open {
                opened.viewer.mark_render_requested(page, width);
            }
            let worker = self.pdf.clone();
            cx.spawn(async move |this, cx| {
                let rendered = worker
                    .render_page(handle, page, width)
                    .await
                    .ok()
                    .and_then(|result| result.ok());
                this.update(cx, |app, cx| match rendered {
                    Some(rendered) => app.record_page_render(handle, page, width, rendered, cx),
                    None => app.forget_page_render(handle, page, width, cx),
                })
                .ok();
            })
            .detach();
        }
        self.pump_thumbnails(cx);
    }

    /// A page render reply arrived: cache it, show it if current, and see
    /// what the view wants next.
    fn record_page_render(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        width: u32,
        rendered: RenderedPage,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.record_render(
                page,
                width,
                Arc::new(canvas::render_image(&rendered.rgba)),
            );
            true
        } else {
            false
        };
        if applied {
            cx.notify();
            self.refresh_view(cx);
        }
    }

    /// A page render failed or belongs to a closed document: drop the
    /// pending slot so a fresh request can be issued.
    fn forget_page_render(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        width: u32,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.clear_pending_render(page, width);
            true
        } else {
            false
        };
        if applied {
            self.refresh_view(cx);
        }
    }

    /// Issues the next queued thumbnail request, one at a time, never
    /// ahead of the current page's render (plan.md §11.2).
    fn pump_thumbnails(&mut self, cx: &mut Context<Self>) {
        loop {
            let next = match &mut self.open {
                OpenState::Opened(opened) => opened.pdf.zip(opened.viewer.next_thumbnail_request()),
                _ => None,
            };
            let Some((handle, page)) = next else {
                break;
            };
            let worker = self.pdf.clone();
            cx.spawn(async move |this, cx| {
                let rendered = worker
                    .render_page(handle, page, crate::viewer::THUMBNAIL_RENDER_WIDTH)
                    .await
                    .ok()
                    .and_then(|result| result.ok());
                this.update(cx, |app, cx| {
                    if let Some(rendered) = rendered {
                        app.record_thumbnail(handle, page, rendered, cx);
                    } else {
                        app.forget_thumbnail(handle, page, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn record_thumbnail(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        rendered: RenderedPage,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened
                .viewer
                .record_thumbnail(page, Arc::new(canvas::render_image(&rendered.rgba)));
            true
        } else {
            false
        };
        if applied {
            cx.notify();
            self.refresh_view(cx);
        }
    }

    fn forget_thumbnail(&mut self, handle: PdfDocumentHandle, page: u32, cx: &mut Context<Self>) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.clear_thumbnail_in_flight(page);
            true
        } else {
            false
        };
        if applied {
            self.refresh_view(cx);
        }
    }

    fn header_title(&self) -> Option<&str> {
        match &self.open {
            OpenState::Opened(opened) => opened.session().source().path().file_name()?.to_str(),
            _ => None,
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let zoom_percent = match &self.open {
            OpenState::Opened(opened) => opened.viewer.zoom_percent(),
            _ => None,
        };

        focus_scope("mark")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &OpenDocument, _window, cx| {
                this.open_with_dialog(cx);
            }))
            .on_action(
                cx.listener(|this, _: &NextPageAction, _window, cx| this.handle_next_page(cx)),
            )
            .on_action(cx.listener(|this, _: &PreviousPageAction, _window, cx| {
                this.handle_previous_page(cx);
            }))
            .on_action(cx.listener(|this, _: &FirstPageAction, _window, cx| {
                this.handle_first_page(cx);
            }))
            .on_action(
                cx.listener(|this, _: &LastPageAction, _window, cx| this.handle_last_page(cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomInAction, _window, cx| this.handle_zoom_in(cx)))
            .on_action(cx.listener(|this, _: &ZoomOutAction, _window, cx| this.handle_zoom_out(cx)))
            .on_action(cx.listener(|this, _: &ZoomFitAction, _window, cx| this.handle_zoom_fit(cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.inset)
            .child(header(&theme, self.header_title(), zoom_percent, cx))
            .child(match &mut self.open {
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
                OpenState::Opened(opened) => workspace(&theme, opened, window, cx),
            })
            .children(match &self.open {
                OpenState::Opened(opened) => Some(status_bar(
                    &theme,
                    opened.viewer.current_page(),
                    opened.viewer.page_count(),
                    opened.viewer.has_previous_page(),
                    opened.viewer.has_next_page(),
                    zoom_percent,
                    cx,
                )),
                _ => None,
            })
    }
}

/// The document workspace: thumbnail sidebar beside the canvas stage
/// (plan.md §10; the assets panel arrives in Phase 6).
fn workspace(
    theme: &Theme,
    opened: &mut OpenedDocument,
    window: &mut Window,
    cx: &mut Context<MarkApp>,
) -> gpui_kit::AnyElement {
    let weak = cx.weak_entity();
    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .items_stretch()
        .overflow_hidden()
        .child(thumbnails::sidebar(theme, &opened.viewer, window, cx))
        .child(canvas::stage(theme, &weak, &mut opened.viewer, cx))
        .into_any_element()
}

fn header(
    theme: &Theme,
    title: Option<&str>,
    zoom_percent: Option<u32>,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
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
        .child(
            div()
                .id("mark-header-actions")
                .flex()
                .items_center()
                .gap(rems(0.375))
                .children(zoom_percent.map(|percent| {
                    div()
                        .id("mark-zoom-controls")
                        .flex()
                        .items_center()
                        .gap(rems(0.25))
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-out",
                                IconName::ZoomOut,
                                "Zoom out",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_out(cx)),
                            ),
                            "Zoom out (-)",
                        ))
                        .child(
                            div()
                                .id("mark-zoom-percent")
                                .text_size(rems(0.75))
                                .text_color(theme.secondary)
                                .min_w(rems(2.5))
                                .flex()
                                .justify_center()
                                .child(format!("{percent}%")),
                        )
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-in",
                                IconName::ZoomIn,
                                "Zoom in",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_in(cx)),
                            ),
                            "Zoom in (+)",
                        ))
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-fit",
                                IconName::Maximize,
                                "Fit page",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_fit(cx)),
                            ),
                            "Fit page (0)",
                        ))
                })),
        )
}

/// Status bar: page navigation and indication on the left, zoom on the
/// right (plan.md §10).
fn status_bar(
    theme: &Theme,
    current_page: u32,
    page_count: usize,
    has_previous: bool,
    has_next: bool,
    zoom_percent: Option<u32>,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    div()
        .id("mark-status-bar")
        .flex()
        .items_center()
        .justify_between()
        .px(rems(1.))
        .h(rems(2.))
        .flex_shrink_0()
        .bg(theme.background)
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .id("mark-status-pages")
                .flex()
                .items_center()
                .gap(rems(0.25))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "first-page",
                        IconName::ChevronsLeft,
                        "First page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_previous)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.handle_first_page(cx)),
                    ),
                    "First page (Home)",
                ))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "previous-page",
                        IconName::ChevronLeft,
                        "Previous page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_previous)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.handle_previous_page(cx)),
                    ),
                    "Previous page (Page Up)",
                ))
                .child(
                    div()
                        .id("mark-status-page-indicator")
                        .px(rems(0.5))
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(format!("Page {} / {}", current_page + 1, page_count)),
                )
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "next-page",
                        IconName::ChevronRight,
                        "Next page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_next)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_next_page(cx))),
                    "Next page (Page Down)",
                ))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "last-page",
                        IconName::ChevronsRight,
                        "Last page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_next)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_last_page(cx))),
                    "Last page (End)",
                )),
        )
        .child(
            div()
                .id("mark-status-zoom")
                .text_size(rems(0.75))
                .text_color(theme.secondary)
                .child(zoom_percent.map(|p| format!("{p}%")).unwrap_or_default()),
        )
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
