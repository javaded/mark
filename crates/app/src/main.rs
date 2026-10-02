//! Mark — desktop document signer (application shell).

mod app;

use app::MarkApp;
use gpui_kit::{
    AppContext as _, Bounds, KeyBinding, Pixels, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

gpui_kit::actions!(mark, [Quit, OpenDocument]);

fn main() {
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
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            // open_window mounts the base Root, which later renders
            // dialogs, sheets, and notifications above the app view.
            gpui_kit::open_window(
                window_options(SharedString::from("Mark"), cx),
                cx,
                |_, cx| cx.new(|_| MarkApp::default()),
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
