//! UI integration tests (plan.md §20.4): the production view in a headless
//! window, driven by synthetic pointer and keyboard events, asserting on
//! the document model — never on screenshots.
//!
//! Phase 7 coverage: asset placement by click, click-select, click-away,
//! Escape, drag-move with one committed `MoveObject`, corner resize with
//! aspect lock (`ResizeObject`), Delete, and Ctrl+Z / Ctrl+Shift+Z.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, point, px, size,
};
use mark_core::{AssetId, AssetKind, ImageObject, ObjectId, ObjectKind};
use mark_export::library::AssetLibrary;

use crate::app::MarkApp;
use crate::init_keybindings;

/// A white 400×300 page document and a 320×120 signature asset, library
/// rooted in a throwaway directory.
struct Fixture {
    window: gpui_kit::AnyWindowHandle,
    app: Entity<MarkApp>,
    asset_id: AssetId,
    /// Dropped at test end; keeps the library files alive while the app
    /// entity may still reference them.
    _dir: tempfile::TempDir,
}

fn write_png(path: &std::path::Path, width: u32, height: u32, fill: [u8; 4]) {
    image::RgbaImage::from_pixel(width, height, image::Rgba(fill))
        .save(path)
        .expect("write fixture png");
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let page_path = dir.path().join("page.png");
    write_png(&page_path, 400, 300, [255, 255, 255, 255]);
    let signature_path = dir.path().join("signature.png");
    write_png(&signature_path, 320, 120, [70, 30, 180, 255]);

    let library_root = dir.path().join("library");
    let asset = AssetLibrary::open(&library_root)
        .and_then(|mut library| library.import(&signature_path, AssetKind::Signature))
        .expect("seed library asset");

    cx.update(|cx| {
        gpui_omarchy::init(cx);
        init_keybindings(cx);
    });

    let pdf = Arc::new(mark_pdf::PdfWorker::spawn());
    let root_for_app = library_root.clone();
    let (window, app) = cx
        .update(|cx| {
            gpui_kit::open_window(test_window_options(), cx, |window, cx| {
                let app = cx.new(|cx| MarkApp::with_library_root(pdf, root_for_app, cx));
                window.focus(&app.read(cx).focus_handle().clone(), cx);
                app
            })
        })
        .expect("open test window");

    // Production open path (dialog bypassed): image decode is async.
    app.update(cx, |app, cx| app.open_path(page_path.clone(), cx));
    cx.run_until_parked();

    Fixture {
        window,
        app,
        asset_id: asset.id(),
        _dir: dir,
    }
}

fn test_window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1120.), px(760.)),
        })),
        ..Default::default()
    }
}

// ----- model observation helpers ---------------------------------------------

/// The sole object of page 0 as `(id, geometry)`.
fn sole_object(fx: &Fixture, cx: &mut TestAppContext) -> (ObjectId, ImageObject) {
    fx.app.update(cx, |app, _| {
        let opened = app.opened_document().expect("document open");
        let page = &opened.session().document().pages()[0];
        assert_eq!(page.objects().len(), 1, "exactly one placed object");
        let object = &page.objects()[0];
        let ObjectKind::Image(data) = object.kind();
        (object.id(), data.clone())
    })
}

fn object_count(fx: &Fixture, cx: &mut TestAppContext) -> usize {
    fx.app.update(cx, |app, _| {
        app.opened_document()
            .expect("document open")
            .session()
            .document()
            .pages()[0]
            .objects()
            .len()
    })
}

fn selected(fx: &Fixture, cx: &mut TestAppContext) -> Option<ObjectId> {
    fx.app.update(cx, |app, _| {
        app.opened_document()
            .expect("document open")
            .selected_object()
    })
}

fn zoom(fx: &Fixture, cx: &mut TestAppContext) -> f32 {
    fx.app.update(cx, |app, _| {
        app.opened_document()
            .expect("document open")
            .viewer()
            .transform()
            .expect("measured viewport")
            .zoom
    })
}

fn next_undo(fx: &Fixture, cx: &mut TestAppContext) -> Option<String> {
    fx.app.update(cx, |app, _| {
        app.opened_document()
            .expect("document open")
            .session()
            .undo_manager()
            .next_undo_description()
    })
}

/// Renders a frame; returns a fresh snapshot lookup closure context.
fn with_window<R>(
    fx: &Fixture,
    cx: &mut TestAppContext,
    interact: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.window, |_, window, cx| interact(window, cx))
        .expect("update test window")
}

// ----- tests ------------------------------------------------------------------

#[gpui_kit::test]
fn click_places_asset_selected_ready_to_drag(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });

    let (id, data) = sole_object(&fx, cx);
    assert_eq!(selected(&fx, cx), Some(id));
    // Placed geometry is mark-core's job (tested there); here it must be
    // on-page and aspect-true for the 320×120 asset.
    assert!((data.width / data.height - 320. / 120.).abs() < 1e-3);
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Add image"));
    // Repeated placement adds a second object (no single-slot special case).
    with_window(&fx, cx, |window, cx| {
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    assert_eq!(object_count(&fx, cx), 2);
}

#[gpui_kit::test]
fn selection_clears_on_click_away_and_escape(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    let (id, _) = sole_object(&fx, cx);
    assert_eq!(selected(&fx, cx), Some(id));

    // Click on the canvas padding (no object): selection clears.
    with_window(&fx, cx, |window, cx| {
        window.click_at("mark-canvas-viewport", point(px(5.), px(5.)), cx);
    });
    assert_eq!(selected(&fx, cx), None);

    // Click the object: selected again, resize handles observable.
    with_window(&fx, cx, |window, cx| {
        window.click(format!("mark-object-{id}"), cx);
        assert!(window.try_find(format!("mark-handle-br-{id}")).is_some());
    });
    assert_eq!(selected(&fx, cx), Some(id));

    // Escape clears it (plan.md §15).
    with_window(&fx, cx, |window, cx| {
        window.press("escape", cx);
    });
    assert_eq!(selected(&fx, cx), None);
}

#[gpui_kit::test]
fn drag_moves_object_and_undo_redo_round_trip(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    let (id, before) = sole_object(&fx, cx);

    // Drag the object center 120px right, 40px down (screen px).
    with_window(&fx, cx, |window, cx| {
        let center = window.find(format!("mark-object-{id}")).bounds().center();
        window.drag(center, point(center.x + px(120.), center.y + px(40.)), cx);
    });

    // One gesture = one committed MoveObject (plan.md §14) on top of the
    // placement's AddObject.
    let (_, after) = sole_object(&fx, cx);
    let zoom = zoom(&fx, cx);
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Move object"));
    assert!((after.x - (before.x + 120. / zoom)).abs() < 0.5);
    assert!((after.y - (before.y + 40. / zoom)).abs() < 0.5);

    // Ctrl+Z: back to the placement position exactly.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
    });
    let (_, undone) = sole_object(&fx, cx);
    assert_eq!(undone, before);

    // Ctrl+Shift+Z: the move returns.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-shift-z", cx);
    });
    let (_, redone) = sole_object(&fx, cx);
    assert_eq!(redone, after);

    // A press-and-release without movement commits nothing (no no-op undo).
    with_window(&fx, cx, |window, cx| {
        let center = window.find(format!("mark-object-{id}")).bounds().center();
        window.drag(center, center, cx);
    });
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Move object"));
}

#[gpui_kit::test]
fn handle_drag_resizes_aspect_locked_and_undo_restores(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    let (id, before) = sole_object(&fx, cx);
    let aspect = before.width / before.height;

    // Drag the bottom-right handle 60px right.
    with_window(&fx, cx, |window, cx| {
        let handle = window
            .find(format!("mark-handle-br-{id}"))
            .bounds()
            .center();
        window.drag(handle, point(handle.x + px(60.), handle.y), cx);
    });

    let (_, resized) = sole_object(&fx, cx);
    let zoom = zoom(&fx, cx);
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Resize object"));
    // Width grew by the screen delta scaled into document units; height
    // follows the original aspect; top-left stays anchored.
    assert!((resized.width - (before.width + 60. / zoom)).abs() < 0.5);
    assert!((resized.width / resized.height - aspect).abs() < 1e-3);
    assert_eq!((resized.x, resized.y), (before.x, before.y));

    // Ctrl+Z restores the exact prior geometry.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
    });
    let (_, undone) = sole_object(&fx, cx);
    assert_eq!(undone, before);
}

#[gpui_kit::test]
fn delete_removes_object_and_undo_restores_it(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    let (id, placed) = sole_object(&fx, cx);
    assert_eq!(selected(&fx, cx), Some(id));

    // Delete removes the overlay object only (plan.md §13.2).
    with_window(&fx, cx, |window, cx| {
        window.press("delete", cx);
    });
    assert_eq!(object_count(&fx, cx), 0);
    assert_eq!(selected(&fx, cx), None);

    // Ctrl+Z brings it back with its geometry.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
    });
    let (restored_id, restored) = sole_object(&fx, cx);
    assert_eq!(restored_id, id);
    assert_eq!(restored, placed);

    // Redo deletes again.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-shift-z", cx);
    });
    assert_eq!(object_count(&fx, cx), 0);
}
