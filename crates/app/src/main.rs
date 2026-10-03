//! Mark — desktop document signer (application shell).

mod app;
mod canvas;
mod thumbnails;
mod viewer;

use std::path::PathBuf;

use app::MarkApp;
use gpui_kit::{
    AppContext as _, Bounds, KeyBinding, Pixels, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

gpui_kit::actions!(
    mark,
    [
        Quit,
        OpenDocument,
        NextPage,
        PreviousPage,
        FirstPage,
        LastPage,
        ZoomIn,
        ZoomOut,
        ZoomFit,
    ]
);

fn main() {
    // Optional file argument: `mark picture.png` opens it directly, no dialog.
    let open_path = std::env::args().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            // Initializes gpui-base, Omarchy components, and the system theme
            // (Omarchy theme on Linux; Tokyo Night fallback elsewhere).
            gpui_omarchy::init(cx);

            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new(&format!("{modifier}-q"), Quit, None),
                KeyBinding::new(&format!("{modifier}-o"), OpenDocument, None),
                // Page navigation (plan.md §15).
                KeyBinding::new("pageup", PreviousPage, None),
                KeyBinding::new("pagedown", NextPage, None),
                KeyBinding::new("home", FirstPage, None),
                KeyBinding::new("end", LastPage, None),
                // Zoom: "+"/"=" both zoom in so shifted and unshifted keys
                // work across layouts (plan.md §15).
                KeyBinding::new("=", ZoomIn, None),
                KeyBinding::new("+", ZoomIn, None),
                KeyBinding::new("-", ZoomOut, None),
                KeyBinding::new("0", ZoomFit, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            // open_window mounts the base Root, which later renders
            // dialogs, sheets, and notifications above the app view.
            gpui_kit::open_window(
                window_options(SharedString::from("Mark"), cx),
                cx,
                |window, cx| {
                    let app = cx.new(|cx| {
                        let pdf = std::sync::Arc::new(mark_pdf::PdfWorker::spawn());
                        let mut app = MarkApp::new(pdf, cx);
                        if let Some(path) = open_path {
                            app.open_path(path, cx);
                        }
                        app
                    });
                    // Actions dispatch through the focused node: give the
                    // app keyboard focus from the first frame.
                    window.focus(&app.read(cx).focus_handle().clone(), cx);
                    app
                },
            )
            .expect("failed to open window");

            cx.activate(true);
        });
}

fn window_options(title: SharedString, cx: &mut gpui_kit::App) -> WindowOptions {
    let min_width: Pixels = px(780.);
    let min_height: Pixels = px(540.);
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title),
            ..Default::default()
        }),
        window_min_size: Some(size(min_width, min_height)),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1120.), px(760.)),
            cx,
        ))),
        ..Default::default()
    }
}
