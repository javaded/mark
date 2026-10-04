//! UI integration tests (plan.md §20.4): the production view in a headless
//! window, driven by synthetic pointer and keyboard events, asserting on
//! the document model — never on screenshots.
//!
//! Phase 7 coverage: asset placement by click, click-select, click-away,
//! Escape, drag-move with one committed `MoveObject`, corner resize with
//! aspect lock (`ResizeObject`), Delete, and Ctrl+Z / Ctrl+Shift+Z.
//!
//! Phase 8 coverage: duplicate (Ctrl+D), copy/paste with cascading offset,
//! the contextual toolbar (size steps, visibility), "Duplicate to page…"
//! through the real page menu, and multiple assets on one page.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, point, px, size,
};
use mark_core::{
    AssetId, AssetKind, Document, DocumentSource, ImageObject, ObjectId, ObjectKind, Page,
};
use mark_export::library::AssetLibrary;

use crate::app::MarkApp;
use crate::init_keybindings;

/// A white 400×300 page document and a 320×120 signature asset, library
/// rooted in a throwaway directory.
struct Fixture {
    window: gpui_kit::AnyWindowHandle,
    app: Entity<MarkApp>,
    asset_id: AssetId,
    /// A second, differently-shaped stamp asset.
    stamp_id: AssetId,
    /// The page image path (documents injected for tests use it as source).
    page_path: std::path::PathBuf,
    /// Dropped at test end; keeps the library files alive while the app
    /// entity may still reference them.
    _dir: tempfile::TempDir,
}

fn write_png(path: &std::path::Path, width: u32, height: u32, fill: [u8; 4]) {
    image::RgbaImage::from_pixel(width, height, image::Rgba(fill))
        .save(path)
        .expect("write fixture png");
}

/// The library, window, and app without any document open.
fn seeded(cx: &mut TestAppContext) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let page_path = dir.path().join("page.png");
    write_png(&page_path, 400, 300, [255, 255, 255, 255]);
    let signature_path = dir.path().join("signature.png");
    write_png(&signature_path, 320, 120, [70, 30, 180, 255]);
    let stamp_path = dir.path().join("stamp.png");
    write_png(&stamp_path, 120, 120, [20, 150, 160, 255]);

    let library_root = dir.path().join("library");
    let mut library = AssetLibrary::open(&library_root).expect("open seed library");
    let asset = library
        .import(&signature_path, AssetKind::Signature)
        .expect("seed signature asset");
    let stamp = library
        .import(&stamp_path, AssetKind::Stamp)
        .expect("seed stamp asset");

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

    Fixture {
        window,
        app,
        asset_id: asset.id(),
        stamp_id: stamp.id(),
        page_path,
        _dir: dir,
    }
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    // Production open path (dialog bypassed): image decode is async.
    let fx = seeded(cx);
    let path = fx.page_path.clone();
    fx.app.update(cx, |app, cx| app.open_path(path, cx));
    cx.run_until_parked();
    fx
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

/// All image objects of `page` as `(id, geometry)` (model observation).
fn page_objects(
    fx: &Fixture,
    cx: &mut TestAppContext,
    page: usize,
) -> Vec<(ObjectId, ImageObject)> {
    fx.app.update(cx, |app, _| {
        app.opened_document()
            .expect("document open")
            .session()
            .document()
            .pages()[page]
            .objects()
            .iter()
            .map(|object| {
                let ObjectKind::Image(data) = object.kind();
                (object.id(), data.clone())
            })
            .collect()
    })
}

/// Places the signature asset on the current page and returns its geometry.
fn place_signature(fx: &Fixture, cx: &mut TestAppContext) -> (ObjectId, ImageObject) {
    with_window(fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
    });
    let (id, data) = sole_object(fx, cx);
    assert_eq!(selected(fx, cx), Some(id));
    (id, data)
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

// ----- Phase 8: multi-object / multi-page workflow -------------------------

#[gpui_kit::test]
fn ctrl_d_duplicates_offset_and_selects_the_copy(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let (id, before) = place_signature(&fx, cx);

    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-d", cx);
    });

    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 2);
    let (duplicate_id, duplicate) = objects[1].clone();
    assert_ne!(duplicate_id, id, "duplicate gets a fresh id");
    assert_eq!(duplicate.asset_id, before.asset_id);
    assert!((duplicate.x - (before.x + 16.)).abs() < 1e-3);
    assert!((duplicate.y - (before.y + 16.)).abs() < 1e-3);
    assert_eq!(
        (duplicate.width, duplicate.height),
        (before.width, before.height)
    );
    assert_eq!(selected(&fx, cx), Some(duplicate_id));
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Duplicate object"));

    // Undo removes the copy only.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
    });
    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].1, before);
}

#[gpui_kit::test]
fn copy_paste_cascades_offset_and_undo_removes_pastes(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let (_, before) = place_signature(&fx, cx);

    // Copy alone changes nothing.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-c", cx);
    });
    assert_eq!(object_count(&fx, cx), 1);

    // First paste: one step offset from the copied position, selected.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-v", cx);
    });
    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 2);
    assert_eq!(selected(&fx, cx), Some(objects[1].0));
    assert!((objects[1].1.x - (before.x + 16.)).abs() < 1e-3);
    assert!((objects[1].1.y - (before.y + 16.)).abs() < 1e-3);
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Add image"));

    // Second paste cascades one step further (plan.md §13.2: never exactly
    // on top).
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-v", cx);
    });
    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 3);
    assert!((objects[2].1.x - (before.x + 32.)).abs() < 1e-3);
    assert!((objects[2].1.y - (before.y + 32.)).abs() < 1e-3);

    // Undo twice: back to just the placed original.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
        window.press("ctrl-z", cx);
    });
    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].1, before);
}

#[gpui_kit::test]
fn toolbar_size_steps_commit_one_resize_each(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let (_, before) = place_signature(&fx, cx);
    let aspect = before.width / before.height;

    // The toolbar exists while an object is selected.
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mark-selection-toolbar").is_some());
        window.click("mark-toolbar-bigger", cx);
    });

    let (_, bigger) = sole_object(&fx, cx);
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Resize object"));
    assert!((bigger.width - before.width * 1.1).abs() < 1e-2);
    assert!((bigger.width / bigger.height - aspect).abs() < 1e-3);
    // Grown about the center: the midpoint stays put.
    assert!((bigger.x + bigger.width / 2. - (before.x + before.width / 2.)).abs() < 1e-2);

    with_window(&fx, cx, |window, cx| {
        window.click("mark-toolbar-smaller", cx);
    });
    let (_, smaller) = sole_object(&fx, cx);
    assert!((smaller.width - bigger.width * 0.9).abs() < 1e-2);

    // Escape clears the selection and the toolbar with it.
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("mark-selection-toolbar").is_none());
    });

    // Undo both steps back to the placed geometry.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
        window.press("ctrl-z", cx);
    });
    let (_, undone) = sole_object(&fx, cx);
    assert_eq!(undone, before);
}

#[gpui_kit::test]
fn toolbar_duplicate_button_and_delete_work_together(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let _ = place_signature(&fx, cx);

    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click("mark-toolbar-duplicate", cx);
    });
    assert_eq!(object_count(&fx, cx), 2);

    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click("mark-toolbar-delete", cx);
    });
    // The selected duplicate is gone; the original remains.
    assert_eq!(object_count(&fx, cx), 1);
    assert_eq!(selected(&fx, cx), None);
}

#[gpui_kit::test]
fn multiple_assets_place_independently(cx: &mut TestAppContext) {
    let fx = fixture(cx);

    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.asset_id), cx);
        window.click(format!("mark-asset-{}", fx.stamp_id), cx);
    });

    let objects = page_objects(&fx, cx, 0);
    assert_eq!(objects.len(), 2);
    assert_ne!(objects[0].1.asset_id, objects[1].1.asset_id);
    // Each placement keeps its own asset's aspect (320×120 vs 120×120).
    assert!((objects[0].1.width / objects[0].1.height - 320. / 120.).abs() < 1e-3);
    assert!((objects[1].1.width / objects[1].1.height - 1.).abs() < 1e-3);
    // The stamp placement is the selected one.
    assert_eq!(selected(&fx, cx), Some(objects[1].0));
}

#[gpui_kit::test]
fn duplicate_to_page_menu_copies_to_the_chosen_page(cx: &mut TestAppContext) {
    let fx = seeded(cx);
    // Three pages, the middle one smaller so clamping is observable: a
    // 100-wide object centered on page 1 pulls back to x = 20 there.
    let document = Document::new(
        DocumentSource::Image {
            path: fx.page_path.clone(),
        },
        vec![
            Page::new(400., 300.),
            Page::new(120., 80.),
            Page::new(400., 300.),
        ],
    );
    fx.app
        .update(cx, |app, cx| app.open_test_document(document, cx));
    cx.run_until_parked();

    let (id, placed) = place_signature(&fx, cx);
    assert_eq!(page_objects(&fx, cx, 0).len(), 1);

    // Open the page menu and pick "Page 2" (row index 1; the current page
    // is row 0, disabled). The popup fades in from transparent, so let the
    // fade finish before clicking inside it.
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click("mark-toolbar-to-page-trigger", cx);
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(("menu-item", 1usize), cx);
    });

    // The copy lands fully on the smaller target page, same size; the
    // source page is untouched and keeps the selection.
    assert_eq!(page_objects(&fx, cx, 0).len(), 1);
    let copies = page_objects(&fx, cx, 1);
    assert_eq!(copies.len(), 1);
    let (_, copy) = copies[0].clone();
    assert_eq!((copy.width, copy.height), (placed.width, placed.height));
    assert!(
        (copy.x - (120. - placed.width)).abs() < 1e-2,
        "clamped onto the target page"
    );
    assert!(
        (copy.y - (80. - placed.height)).abs() < 1e-2,
        "clamped onto the target page"
    );
    assert_eq!(page_objects(&fx, cx, 2).len(), 0);
    assert_eq!(selected(&fx, cx), Some(id));
    assert_eq!(next_undo(&fx, cx).as_deref(), Some("Duplicate to page"));

    // Undo removes only the copy.
    with_window(&fx, cx, |window, cx| {
        window.press("ctrl-z", cx);
    });
    assert_eq!(page_objects(&fx, cx, 0).len(), 1);
    assert_eq!(page_objects(&fx, cx, 1).len(), 0);
}

// ----- export (plan.md §16, §20.3) ----------------------------------------------

/// Waits for the export to leave the `Running` state.
///
/// PDF exports run on the real PDFium worker thread, which wakes the
/// foreground task from outside the test thread. With the scheduler's
/// `allow_parking` opt-in enabled (see the callers), pump the executor
/// in small steps until the status settles — bounded, so a dead worker
/// fails the assertions instead of hanging the suite.
fn wait_until_export_settles(fx: &Fixture, cx: &mut TestAppContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        let settled = fx.app.update(cx, |app, _| {
            !matches!(
                app.export_status(),
                Some(crate::app::ExportStatus::Running { .. })
            )
        });
        if settled || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A test PDF fixture path under `resources/test-documents/`.
fn pdf_fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../resources/test-documents")
        .join(name)
}

#[gpui_kit::test]
fn export_image_document_writes_composed_png(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let (_, placed) = place_signature(&fx, cx);
    let original_bytes = std::fs::read(&fx.page_path).expect("read original");

    let destination = fx._dir.path().join("page-signed.png");
    fx.app
        .update(cx, |app, cx| app.export_to(destination.clone(), cx));
    cx.run_until_parked();

    // Outcome: done, and the session's dirty state resets (mark_saved).
    let done = fx.app.update(cx, |app, _| {
        matches!(
            app.export_status(),
            Some(crate::app::ExportStatus::Done { .. })
        )
    });
    assert!(done);
    fx.app.update(cx, |app, _| {
        assert!(!app.opened_document().unwrap().session().is_dirty());
    });

    // The exported PNG: the signature composited at its placement rect,
    // the base image elsewhere (never a canvas screenshot, §16.1).
    let exported = image::open(&destination)
        .expect("exported png readable")
        .to_rgba8();
    assert_eq!(exported.dimensions(), (400, 300));
    let inside = exported.get_pixel(
        (placed.x + placed.width / 2.) as u32,
        (placed.y + placed.height / 2.) as u32,
    );
    assert_eq!(inside, &image::Rgba([70, 30, 180, 255]), "signature pixels");
    let outside = exported.get_pixel(10, 10);
    assert_eq!(outside, &image::Rgba([255, 255, 255, 255]), "base pixels");

    // The original file is byte-identical (plan.md §16).
    assert_eq!(
        std::fs::read(&fx.page_path).expect("re-read original"),
        original_bytes
    );
}

#[gpui_kit::test]
fn export_fails_loudly_when_an_asset_file_is_gone(cx: &mut TestAppContext) {
    let fx = fixture(cx);
    let _ = place_signature(&fx, cx);

    // The library still lists the asset, but its normalized PNG was
    // deleted outside the app: the export must fail visibly, never write
    // a signed copy that silently misses a signature.
    let asset_file = fx
        ._dir
        .path()
        .join("library")
        .join(format!("assets/{0}.png", fx.asset_id));
    std::fs::remove_file(&asset_file).expect("remove asset png");

    let destination = fx._dir.path().join("never-written.png");
    fx.app
        .update(cx, |app, cx| app.export_to(destination.clone(), cx));
    cx.run_until_parked();

    let failed = fx.app.update(cx, |app, _| {
        matches!(
            app.export_status(),
            Some(crate::app::ExportStatus::Failed(_))
        )
    });
    assert!(failed);
    assert!(!destination.exists(), "a failed export writes nothing");
}

#[gpui_kit::test]
fn export_pdf_document_places_real_image_objects(cx: &mut TestAppContext) {
    // Same graceful skip as mark-pdf's integration tests when no runtime.
    let Some(pdfium) = mark_pdf::bind::try_bind() else {
        return;
    };
    // This test drives the real PDFium worker thread: its load/render/
    // export replies wake foreground tasks from off the test thread, so
    // opt the scheduler into that mix (see wait_until_export_settles).
    cx.executor().allow_parking();
    let fx = seeded(cx);
    let path = pdf_fixture("letter-portrait.pdf");
    fx.app.update(cx, |app, cx| app.open_path(path, cx));
    cx.run_until_parked();

    // Sign page 1 with the stamp through the real placement path.
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        window.click(format!("mark-asset-{}", fx.stamp_id), cx);
    });
    let (_, placed) = sole_object(&fx, cx);
    let original_bytes = std::fs::read(pdf_fixture("letter-portrait.pdf")).expect("read original");

    let destination = fx._dir.path().join("letter-signed.pdf");
    fx.app
        .update(cx, |app, cx| app.export_to(destination.clone(), cx));
    wait_until_export_settles(&fx, cx);

    let done = fx.app.update(cx, |app, _| {
        matches!(
            app.export_status(),
            Some(crate::app::ExportStatus::Done { .. })
        )
    });
    assert!(done, "pdf export finished");

    // Reload the export through the direct binding: the placement became a
    // real image page object at the display rect mapped to user space
    // (letter page, no rotation: user_y = 792 − y − h).
    use pdfium_render::prelude::{PdfPageObjectCommon as _, PdfPageObjectsCommon as _};
    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload exported pdf");
    let page = pdf.pages().get(0).expect("page 0");
    let mut image_objects = 0;
    for object in page.objects().iter() {
        if let Some(image) = object.as_image_object() {
            image_objects += 1;
            let bounds = image.bounds().expect("image bounds");
            assert!(
                (bounds.left().value - placed.x).abs() < 0.5,
                "left: {} vs {}",
                bounds.left().value,
                placed.x
            );
            assert!(
                (bounds.bottom().value - (792. - placed.y - placed.height)).abs() < 0.5,
                "bottom: {} vs {}",
                bounds.bottom().value,
                792. - placed.y - placed.height
            );
            assert!(
                (bounds.width().value - placed.width).abs() < 0.5,
                "width: {} vs {}",
                bounds.width().value,
                placed.width
            );
            assert!(
                (bounds.height().value - placed.height).abs() < 0.5,
                "height: {} vs {}",
                bounds.height().value,
                placed.height
            );
        }
    }
    assert_eq!(image_objects, 1, "exactly the placed object");

    // The source PDF is untouched.
    assert_eq!(
        std::fs::read(pdf_fixture("letter-portrait.pdf")).expect("re-read original"),
        original_bytes
    );
}

#[gpui_kit::test]
fn export_button_and_progress_surface_exist(cx: &mut TestAppContext) {
    let fx = fixture(cx);

    // No document: no export button.
    let empty = seeded(cx);
    with_window(&empty, cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mark-export-button").is_none());
    });

    // With a document: the button is there and clickable; the status bar
    // carries the export feedback surface.
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mark-export-button").is_some());
        assert!(window.try_find("mark-export-status").is_none());
    });
    let (_, _) = place_signature(&fx, cx);
    let destination = fx._dir.path().join("page-signed.png");
    fx.app
        .update(cx, |app, cx| app.export_to(destination.clone(), cx));
    cx.run_until_parked();
    with_window(&fx, cx, |window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mark-export-status").is_some());
    });
}
