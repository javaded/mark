//! Main application view: header bar, workspace states, open flow.
//!
//! Phase 3: images open as one-page documents and render on the canvas
//! (plan.md §24). Thumbnails, PDF pages, and the asset panel arrive in
//! later phases.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{
    ClickEvent, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, rems,
};
use gpui_omarchy::{ActiveTheme, ButtonVariant, IconName, Theme, button, focus_scope, icon};
use mark_core::DocumentSession;

use crate::OpenDocument;
use crate::canvas;

/// Everything needed to display an opened image document.
pub struct OpenedDocument {
    session: DocumentSession,
    image: Arc<gpui_kit::RenderImage>,
}

impl OpenedDocument {
    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    pub fn image(&self) -> &Arc<gpui_kit::RenderImage> {
        &self.image
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
}

impl MarkApp {
    pub fn new() -> Self {
        Self {
            open: OpenState::Empty,
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

    /// Loads `path` as a document, decoding off the UI thread.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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
                            image: Arc::new(canvas::render_image(&rgba)),
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
                OpenState::Opened(opened) => {
                    canvas::page(&theme, opened.image()).into_any_element()
                }
            })
    }
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
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
