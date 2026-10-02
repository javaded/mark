//! Main application view: header bar and empty workspace.
//!
//! Phase 1: shell only. The document canvas, page thumbnails, and asset
//! panel arrive in Phases 3-6 (see plan.md §24).

use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, rems,
};
use gpui_omarchy::{ActiveTheme, IconName, Theme, focus_scope, icon};

use crate::OpenDocument;

pub struct MarkApp {
    /// Transient message shown in the empty workspace.
    status: SharedString,
}

impl Default for MarkApp {
    fn default() -> Self {
        Self {
            status: "Press {open} to open a PDF or image.".into(),
        }
    }
}

impl MarkApp {
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
        let theme = cx.omarchy();
        let status = self.status.replace("{open}", Self::open_hint());

        focus_scope("mark")
            .on_action(cx.listener(|this, _: &OpenDocument, _window, cx| {
                this.status = "Opening documents arrives in Phase 3 — shortcut is live.".into();
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.inset)
            .child(header(theme))
            .child(empty_workspace(theme, &status))
    }
}

fn header(theme: &Theme) -> impl IntoElement {
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
                        .child("Sign anything."),
                ),
        )
        // Zoom controls and Export button arrive with the document canvas (Phase 5+).
        .child(div().id("mark-header-actions"))
}

fn empty_workspace(theme: &Theme, status: &str) -> impl IntoElement {
    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
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
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(status.to_owned()),
                ),
        )
}
