//! Mark — desktop document signer (application shell).

mod app;
mod assets;
mod canvas;
mod manipulation;
mod thumbnails;
#[cfg(test)]
mod ui_tests;
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
        Export,
        NextPage,
        PreviousPage,
        FirstPage,
        LastPage,
        ZoomIn,
        ZoomOut,
        ZoomFit,
        ClearSelection,
        DeleteSelected,
        Copy,
        Paste,
        Duplicate,
        Undo,
        Redo,
    ]
);

fn main() {
    // Structured logging (plan.md §17): never document contents or
    // signature pixels — counts, durations, and outcomes only.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    // Optional file argument: `mark picture.png` opens it directly, no dialog.
    let open_path = std::env::args().nth(1).map(PathBuf::from);

    gpui_kit::application()
        // The full Lucide catalog, not the curated default subset: Mark
        // uses icons (signature, stamp, zoom, chevrons, file-x) outside
        // gpui-kit's own component set — with the subset they silently
        // render as nothing.
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            // Initializes gpui-base, Omarchy components, and the system theme
            // (Omarchy theme on Linux; Tokyo Night fallback elsewhere).
            gpui_omarchy::init(cx);

            init_keybindings(cx);

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
                    // Closing the window goes through the same dirty-state
                    // decision as quitting (plan.md §16); the app refuses
                    // while a document has unexported changes.
                    let close_app = app.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        close_app
                            .update(cx, |app, cx| app.request_close(window, cx))
                            .unwrap_or(true)
                    });
                    app
                },
            )
            .expect("failed to open window");

            cx.activate(true);
        });
}

/// Production keybindings (plan.md §15); tests reuse the same table so
/// synthetic keys exercise the real dispatch path.
fn init_keybindings(cx: &mut gpui_kit::App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-q"), Quit, None),
        KeyBinding::new(&format!("{modifier}-o"), OpenDocument, None),
        // Export (plan.md §15: Ctrl+S is export — the MVP has no separate
        // save-to-output-path semantics).
        KeyBinding::new(&format!("{modifier}-s"), Export, None),
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
        // Selection + editing (plan.md §15).
        KeyBinding::new("escape", ClearSelection, None),
        KeyBinding::new("delete", DeleteSelected, None),
        KeyBinding::new("backspace", DeleteSelected, None),
        KeyBinding::new(&format!("{modifier}-c"), Copy, None),
        KeyBinding::new(&format!("{modifier}-v"), Paste, None),
        KeyBinding::new(&format!("{modifier}-d"), Duplicate, None),
        KeyBinding::new(&format!("{modifier}-z"), Undo, None),
        KeyBinding::new(&format!("{modifier}-shift-z"), Redo, None),
        KeyBinding::new(&format!("{modifier}-y"), Redo, None),
    ]);
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
