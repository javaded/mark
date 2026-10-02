//! Mark — desktop document signer (application shell).
//!
//! Phase 0: window bootstrap only. Layout, canvas, and panels arrive in
//! later phases (see plan.md §24).

use gpui_kit::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowOptions, div,
};
use gpui_omarchy::{ActiveTheme, panel};

struct MarkApp;

impl Render for MarkApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.omarchy().background)
            .child(panel("Mark", cx).child("Sign anything. (Phase 0 — workspace bootstrap)"))
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            // Initializes gpui-base, Omarchy components, and the system theme
            // (Omarchy theme on Linux; Tokyo Night fallback elsewhere).
            gpui_omarchy::init(cx);

            // open_window mounts the base Root, which later renders
            // dialogs, sheets, and notifications above the app view.
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| MarkApp))
                .expect("failed to open window");

            cx.activate(true);
        });
}
