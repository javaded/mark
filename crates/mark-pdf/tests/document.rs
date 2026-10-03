//! PDF loading: page counts, geometry, rotation, crop boxes (plan.md §8.1,
//! §20.3 fixtures).

mod common;

use common::{fixture, pdfium_or_skip};
use mark_core::PageRotation;
use mark_pdf::{LoadPdfError, PdfWorker};
fn load(name: &str) -> mark_pdf::LoadedPdf {
    let worker = PdfWorker::spawn();
    let loaded = pollster::block_on(worker.load(fixture(name)))
        .expect("worker alive")
        .expect("fixture loads");
    worker.close(loaded.handle);
    loaded
}

#[test]
fn letter_portrait_loads_with_plain_geometry() {
    let Some(_) = pdfium_or_skip() else { return };
    let loaded = load("letter-portrait.pdf");

    assert_eq!(loaded.document.page_count(), 1);
    let page = &loaded.document.pages()[0];
    assert_eq!((page.width(), page.height()), (612.0, 792.0));
    assert_eq!(page.rotation(), PageRotation::None);

    let geometry = loaded.pages[0];
    assert_eq!(geometry.origin, mark_core::Vec2::new(0.0, 0.0));
    assert_eq!(geometry.size, mark_core::Vec2::new(612.0, 792.0));
    assert_eq!(geometry.display_size(), mark_core::Vec2::new(612.0, 792.0));
}

#[test]
fn a4_landscape_reports_landscape_geometry() {
    let Some(_) = pdfium_or_skip() else { return };
    let loaded = load("a4-landscape.pdf");

    let geometry = loaded.pages[0];
    assert_eq!(geometry.size, mark_core::Vec2::new(842.0, 595.0));
    assert_eq!(geometry.display_size(), mark_core::Vec2::new(842.0, 595.0));
    assert_eq!(
        (
            loaded.document.pages()[0].width(),
            loaded.document.pages()[0].height()
        ),
        (842.0, 595.0)
    );
}

/// The highest-risk case (plan.md §27.1): a portrait MediaBox with /Rotate 90
/// displays as landscape. Page model and geometry must both say so.
#[test]
fn rotate_90_page_displays_landscape() {
    let Some(_) = pdfium_or_skip() else { return };
    let loaded = load("rotated-90.pdf");

    // User-space box stays portrait…
    let geometry = loaded.pages[0];
    assert_eq!(geometry.size, mark_core::Vec2::new(612.0, 792.0));
    assert_eq!(geometry.rotation, PageRotation::Degrees90);
    // …while the displayed page is landscape.
    assert_eq!(geometry.display_size(), mark_core::Vec2::new(792.0, 612.0));
    assert_eq!(
        (
            loaded.document.pages()[0].width(),
            loaded.document.pages()[0].height()
        ),
        (792.0, 612.0)
    );
    assert_eq!(
        loaded.document.pages()[0].rotation(),
        PageRotation::Degrees90
    );
}

#[test]
fn crop_box_geometry_reports_origin_and_size() {
    let Some(_) = pdfium_or_skip() else { return };
    let loaded = load("cropped.pdf");

    let geometry = loaded.pages[0];
    assert_eq!(geometry.origin, mark_core::Vec2::new(61.2, 79.2));
    assert_eq!(geometry.size, mark_core::Vec2::new(489.6, 633.6));
    assert_eq!(geometry.display_size(), mark_core::Vec2::new(489.6, 633.6));
}

#[test]
fn mixed_sizes_load_page_per_page_geometry() {
    let Some(_) = pdfium_or_skip() else { return };
    let loaded = load("mixed-sizes.pdf");

    assert_eq!(loaded.document.page_count(), 3);
    let expected = [(612.0, 792.0), (792.0, 612.0), (500.0, 400.0)];
    for (page, (width, height)) in loaded.document.pages().iter().zip(expected) {
        assert_eq!((page.width(), page.height()), (width, height));
        assert_eq!(page.rotation(), PageRotation::None);
    }
}

#[test]
fn not_a_pdf_reports_open_error() {
    let Some(_) = pdfium_or_skip() else { return };
    let mut path = std::env::temp_dir();
    path.push(format!("mark-pdf-notapdf-{}.txt", std::process::id()));
    std::fs::write(&path, "definitely not a pdf").unwrap();

    let worker = PdfWorker::spawn();
    let error = pollster::block_on(worker.load(path))
        .expect("worker alive")
        .unwrap_err();
    assert!(matches!(error, LoadPdfError::Open(_)));
}
