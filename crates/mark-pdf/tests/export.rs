//! Export integration tests (plan.md §20.3): load → add a known object →
//! export → reload → assert page count, page sizes, and object placement
//! at known coordinates. Never rely on "returned Ok".
//!
//! The export itself runs through the real worker request path; the
//! reloaded file is inspected through a direct runtime binding (page
//! objects are worker-private) and re-rendered for pixel assertions.
//!
//! The overlay is deliberately asymmetric — red top half, blue bottom half
//! — so re-rendering the exported page pins both *where* it lands and
//! *that it is upright*, through every `/Rotate` and crop box.

mod common;

use common::{assert_color, fixture, pdfium_or_skip, sample};
use image::{Rgba, RgbaImage};
use mark_core::{PageCoordinateMapper, Rect, Vec2};
use mark_pdf::{ImageOverlay, PdfExport, PdfWorker};
use pdfium_render::prelude::{
    PdfBitmap, PdfPage, PdfPageObjectCommon as _, PdfPageObjectsCommon as _, PdfRenderConfig,
    Pdfium,
};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// 120×60 PNG: red top half, blue bottom half (orientation probe).
fn orientation_png() -> Vec<u8> {
    let image = RgbaImage::from_fn(120, 60, |_, y| {
        if y < 30 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode test overlay");
    bytes
}

fn overlay(x: f32, y: f32, w: f32, h: f32) -> ImageOverlay {
    ImageOverlay {
        rect: Rect {
            x,
            y,
            width: w,
            height: h,
        },
        png: orientation_png(),
    }
}

/// Exports `name` with `overlays` through the real worker path and returns
/// (destination, progress events). The destination is unique per call —
/// tests run concurrently in one process, and a shared pid-derived path
/// had two exports racing to write the same file.
fn export_via_worker(
    worker: &PdfWorker,
    name: &str,
    overlays: Vec<Vec<ImageOverlay>>,
) -> (std::path::PathBuf, Vec<(u32, u32)>) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static EXPORT_N: AtomicU32 = AtomicU32::new(0);
    let source = fixture(name);
    let destination = std::env::temp_dir().join(format!(
        "mark-export-{}-{}-{}.pdf",
        name.trim_end_matches(".pdf"),
        std::process::id(),
        EXPORT_N.fetch_add(1, Ordering::Relaxed),
    ));
    let loaded = pollster::block_on(worker.load(source.clone()))
        .expect("worker alive")
        .expect("fixture loads");
    worker.close(loaded.handle);

    let (mut progress, result) = worker.export(PdfExport {
        source,
        destination: destination.clone(),
        pages: overlays,
    });
    pollster::block_on(result)
        .expect("worker alive")
        .expect("export succeeds");
    // The worker sends progress while it works and the reply last, so
    // after the reply every event has arrived.
    let mut events = Vec::new();
    while let Ok(mark_pdf::ExportProgress { done, total }) = progress.try_recv() {
        events.push((done, total));
    }
    (destination, events)
}

/// The image objects of `page`, as (left, bottom, right, top) user-space
/// bounds.
fn image_bounds(page: &PdfPage<'_>) -> Vec<(f32, f32, f32, f32)> {
    let mut bounds = Vec::new();
    for object in page.objects().iter() {
        if let Some(image) = object.as_image_object() {
            let rect = image.bounds().expect("image bounds");
            bounds.push((
                rect.left().value,
                rect.bottom().value,
                rect.right().value,
                rect.top().value,
            ));
        }
    }
    bounds
}

/// Renders a page of the exported file at `width` px and samples `(fx, fy)`
/// (fractions of the display size).
fn exported_sample(
    pdfium: &Pdfium,
    path: &std::path::Path,
    index: i32,
    width: i32,
    fx: f64,
    fy: f64,
) -> [u8; 4] {
    let pdf = pdfium
        .load_pdf_from_file(path, None)
        .expect("reload exported");
    let page = pdf.pages().get(index).expect("page exists");
    let bitmap: PdfBitmap = page
        .render_with_config(&PdfRenderConfig::new().set_target_width(width))
        .expect("render exported page");
    let rgba = bitmap.as_image().expect("bitmap to image").to_rgba8();
    sample(&rgba, fx, fy).0
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < 0.5,
        "{context}: expected {expected}, got {actual}"
    );
}

/// Expected user-space (left, bottom, right, top) for a display-space
/// rect on a page with `geometry`.
fn expected_bounds(geometry: &mark_core::PageGeometry, rect: Rect) -> (f32, f32, f32, f32) {
    let mapper: PageCoordinateMapper = geometry.mapper();
    let top_left = mapper.display_to_user(Vec2::new(rect.x, rect.y));
    let bottom_left = mapper.display_to_user(Vec2::new(rect.x, rect.y + rect.height));
    let bottom_right = mapper.display_to_user(Vec2::new(rect.x + rect.width, rect.y + rect.height));
    let top_right = mapper.display_to_user(Vec2::new(rect.x + rect.width, rect.y));
    let xs = [top_left.x, bottom_left.x, bottom_right.x, top_right.x];
    let ys = [top_left.y, bottom_left.y, bottom_right.y, top_right.y];
    (
        xs.iter().copied().fold(f32::MAX, f32::min),
        ys.iter().copied().fold(f32::MAX, f32::min),
        xs.iter().copied().fold(f32::MIN, f32::max),
        ys.iter().copied().fold(f32::MIN, f32::max),
    )
}

/// Page geometries of an exported file, read through the direct binding.
fn geometries(pdfium: &Pdfium, path: &std::path::Path) -> Vec<mark_core::PageGeometry> {
    let pdf = pdfium
        .load_pdf_from_file(path, None)
        .expect("reload exported");
    let mut result = Vec::new();
    for page in pdf.pages().iter() {
        let rotation =
            mark_core::PageRotation::from_degrees(page.rotation().unwrap().as_degrees() as u16);
        let bounds = page
            .boundaries()
            .crop()
            .or_else(|_| page.boundaries().media())
            .unwrap()
            .bounds;
        result.push(mark_core::PageGeometry::new(
            Vec2::new(bounds.left().value, bounds.bottom().value),
            Vec2::new(bounds.width().value, bounds.height().value),
            rotation,
        ));
    }
    result
}

#[test]
fn letter_export_places_overlay_upright_at_known_rect() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let placement = Rect {
        x: 156.,
        y: 371.,
        width: 120.,
        height: 60.,
    };
    let (destination, progress) = export_via_worker(
        &worker,
        "letter-portrait.pdf",
        vec![vec![overlay(
            placement.x,
            placement.y,
            placement.width,
            placement.height,
        )]],
    );

    // Page count and geometry survive (plan.md §16.1).
    let geometries = geometries(pdfium, &destination);
    assert_eq!(geometries.len(), 1);
    assert_eq!(geometries[0].display_size(), Vec2::new(612., 792.));

    // Exactly one image object, at the display rect mapped to user space.
    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    let bounds = image_bounds(&pdf.pages().get(0).unwrap());
    assert_eq!(bounds.len(), 1, "exactly the overlay image object");
    let (left, bottom, right, top) = bounds[0];
    let expected = expected_bounds(&geometries[0], placement);
    assert_close(left, expected.0, "left");
    assert_close(bottom, expected.1, "bottom");
    assert_close(right, expected.2, "right");
    assert_close(top, expected.3, "top");

    // Upright: the display top half of the placement is red, bottom blue.
    let color = exported_sample(pdfium, &destination, 0, 612, 216. / 612., 386. / 792.);
    assert_color(color.into(), RED, "overlay top half is red");
    let color = exported_sample(pdfium, &destination, 0, 612, 216. / 612., 416. / 792.);
    assert_color(color.into(), BLUE, "overlay bottom half is blue");

    // Progress reported the single overlay page.
    assert_eq!(progress, vec![(1, 1)]);
}

#[test]
fn rotated_90_export_places_overlay_upright_through_rotation() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let placement = Rect {
        x: 100.,
        y: 60.,
        width: 120.,
        height: 60.,
    };
    let (destination, _) = export_via_worker(
        &worker,
        "rotated-90.pdf",
        vec![vec![overlay(
            placement.x,
            placement.y,
            placement.width,
            placement.height,
        )]],
    );

    // Rotation metadata and display size survive.
    let geometries = geometries(pdfium, &destination);
    assert_eq!(
        geometries[0].display_size(),
        Vec2::new(792., 612.),
        "rotated page still displays 792×612"
    );

    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    let bounds = image_bounds(&pdf.pages().get(0).unwrap());
    assert_eq!(bounds.len(), 1);
    let (left, bottom, right, top) = bounds[0];
    let expected = expected_bounds(&geometries[0], placement);
    assert_close(left, expected.0, "left");
    assert_close(bottom, expected.1, "bottom");
    assert_close(right, expected.2, "right");
    assert_close(top, expected.3, "top");

    // Upright through the rotation: display top half red, bottom blue.
    let color = exported_sample(pdfium, &destination, 0, 792, 160. / 792., 75. / 612.);
    assert_color(color.into(), RED, "rotated overlay top half is red");
    let color = exported_sample(pdfium, &destination, 0, 792, 160. / 792., 105. / 612.);
    assert_color(color.into(), BLUE, "rotated overlay bottom half is blue");
}

#[test]
fn cropped_export_offsets_by_crop_origin() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let placement = Rect {
        x: 100.,
        y: 100.,
        width: 80.,
        height: 40.,
    };
    let (destination, _) = export_via_worker(
        &worker,
        "cropped.pdf",
        vec![vec![overlay(
            placement.x,
            placement.y,
            placement.width,
            placement.height,
        )]],
    );

    let geometries = geometries(pdfium, &destination);
    let display = geometries[0].display_size();
    assert!(
        (display.x - 489.6).abs() < 0.5 && (display.y - 633.6).abs() < 0.5,
        "cropped display size: {display:?}"
    );

    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    let bounds = image_bounds(&pdf.pages().get(0).unwrap());
    assert_eq!(bounds.len(), 1);
    let (left, bottom, right, top) = bounds[0];
    // Crop origin (61.2, 79.2) shifts every user coordinate.
    assert_close(left, 61.2 + 100., "left");
    assert_close(bottom, 79.2 + 633.6 - 140., "bottom");
    assert_close(right, 61.2 + 180., "right");
    assert_close(top, 79.2 + 633.6 - 100., "top");
}

#[test]
fn mixed_sizes_export_overlays_each_page_and_reports_progress() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let (destination, progress) = export_via_worker(
        &worker,
        "mixed-sizes.pdf",
        vec![
            vec![overlay(50., 50., 100., 50.)],
            vec![],
            vec![overlay(200., 200., 100., 50.)],
        ],
    );

    let geometries = geometries(pdfium, &destination);
    assert_eq!(geometries.len(), 3);
    let expected_sizes = [
        Vec2::new(612., 792.),
        Vec2::new(792., 612.),
        Vec2::new(500., 400.),
    ];
    for (geometry, expected) in geometries.iter().zip(expected_sizes) {
        assert_eq!(geometry.display_size(), expected);
    }

    // Page 0 and page 2 carry one image each; page 1 is untouched.
    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    for index in [0, 1, 2] {
        let page = pdf.pages().get(index).unwrap();
        assert_eq!(
            image_bounds(&page).len(),
            usize::from(index != 1),
            "page {index} image objects"
        );
    }

    // Progress covers both overlay pages, in order, with the true total.
    assert_eq!(progress, vec![(1, 3), (3, 3)]);
}

#[test]
fn a4_landscape_export_places_overlay_upright() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let placement = Rect {
        x: 300.,
        y: 150.,
        width: 140.,
        height: 70.,
    };
    let (destination, _) = export_via_worker(
        &worker,
        "a4-landscape.pdf",
        vec![vec![overlay(
            placement.x,
            placement.y,
            placement.width,
            placement.height,
        )]],
    );

    // Landscape geometry survives (§20.3 fixture).
    let geometries = geometries(pdfium, &destination);
    assert_eq!(geometries.len(), 1);
    assert_eq!(geometries[0].display_size(), Vec2::new(842., 595.));

    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    let bounds = image_bounds(&pdf.pages().get(0).unwrap());
    assert_eq!(bounds.len(), 1);
    let (left, bottom, right, top) = bounds[0];
    let expected = expected_bounds(&geometries[0], placement);
    assert_close(left, expected.0, "left");
    assert_close(bottom, expected.1, "bottom");
    assert_close(right, expected.2, "right");
    assert_close(top, expected.3, "top");

    // Upright on the landscape page: top half red, bottom blue.
    let color = exported_sample(pdfium, &destination, 0, 842, 370. / 842., 166. / 595.);
    assert_color(color.into(), RED, "landscape overlay top half is red");
    let color = exported_sample(pdfium, &destination, 0, 842, 370. / 842., 204. / 595.);
    assert_color(color.into(), BLUE, "landscape overlay bottom half is blue");
}

#[test]
fn text_stays_selectable_after_export() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let (destination, _) = export_via_worker(
        &worker,
        "text-heavy.pdf",
        vec![vec![overlay(156., 371., 120., 60.)]],
    );

    let pdf = pdfium
        .load_pdf_from_file(&destination, None)
        .expect("reload");
    let page = pdf.pages().get(0).unwrap();
    let mut text_objects = 0;
    for object in page.objects().iter() {
        if object.as_text_object().is_some() {
            text_objects += 1;
        }
    }
    assert!(
        text_objects >= 30,
        "text objects survive export as real text ({text_objects} found)"
    );
    assert_eq!(image_bounds(&page).len(), 1, "plus the overlay image");
}

#[test]
fn scanned_page_content_survives_export() {
    let Some(pdfium) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let (destination, _) = export_via_worker(
        &worker,
        "scanned.pdf",
        vec![vec![overlay(246., 366., 120., 60.)]],
    );

    // The scan is still there: the white corner marker and the dark
    // gradient bottom render around the overlay.
    let marker = exported_sample(pdfium, &destination, 0, 612, 0.05, 0.05);
    assert!(
        marker[0] > 200,
        "white corner marker survives, got {marker:?}"
    );
    let dark = exported_sample(pdfium, &destination, 0, 612, 0.5, 0.95);
    assert!(dark[0] < 120, "dark gradient bottom survives, got {dark:?}");

    // And the overlay sits on top where placed.
    let overlay = exported_sample(pdfium, &destination, 0, 612, 306. / 612., 381. / 792.);
    assert_color(overlay.into(), RED, "overlay top half is red");
}

#[test]
fn export_never_touches_the_source_file() {
    let Some(_) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let source = fixture("letter-portrait.pdf");
    let before = std::fs::read(&source).expect("read source");
    let _ = export_via_worker(
        &worker,
        "letter-portrait.pdf",
        vec![vec![overlay(0., 0., 60., 30.)]],
    );
    let after = std::fs::read(&source).expect("re-read source");
    assert_eq!(
        before, after,
        "the original PDF is never modified (plan.md §16)"
    );
}

#[test]
fn export_to_unwritable_destination_errors_cleanly() {
    let Some(_) = pdfium_or_skip() else {
        return;
    };
    let worker = PdfWorker::spawn();
    let (progress, result) = worker.export(PdfExport {
        source: fixture("letter-portrait.pdf"),
        destination: std::path::Path::new("/nonexistent-dir/mark-export.pdf").to_path_buf(),
        pages: vec![vec![overlay(0., 0., 60., 30.)]],
    });
    drop(progress);
    let outcome = pollster::block_on(result).expect("worker alive");
    assert!(
        outcome.is_err(),
        "unwritable destination is an error, not a panic"
    );
}
