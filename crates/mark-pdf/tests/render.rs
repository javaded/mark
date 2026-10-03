//! Rendering: bitmap size, scale, and rotation-aware pixel placement
//! (plan.md §11, §8.1 fixtures).
//!
//! Each fixture paints one colored quadrant in *user space*; where that
//! color lands in the *displayed* bitmap proves the whole rotation chain.

mod common;

use common::{assert_color, fixture, pdfium_or_skip, sample};
use mark_pdf::{PdfWorker, RenderPageError};

fn render(name: &str, page_index: u32, target_width: u32) -> (mark_pdf::RenderedPage, PdfWorker) {
    let worker = PdfWorker::spawn();
    let loaded = pollster::block_on(worker.load(fixture(name)))
        .expect("worker alive")
        .expect("fixture loads");
    let rendered = pollster::block_on(worker.render_page(loaded.handle, page_index, target_width))
        .expect("worker alive")
        .expect("render succeeds");
    worker.close(loaded.handle);
    (rendered, worker)
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const YELLOW: [u8; 3] = [255, 255, 0];

#[test]
fn letter_renders_at_target_width_with_aspect() {
    let Some(_) = pdfium_or_skip() else { return };
    let (rendered, _worker) = render("letter-portrait.pdf", 0, 300);

    assert_eq!(rendered.rgba.width(), 300);
    // 612×792 page at 300px wide → 300/612*792 ≈ 388px tall.
    let expected_height = (300.0_f64 / 612.0 * 792.0).round() as u32;
    assert_eq!(rendered.rgba.height(), expected_height);
    assert!((rendered.scale - 300.0 / 612.0).abs() < 0.01);
}

#[test]
fn letter_red_quadrant_lands_bottom_left_without_rotation() {
    let Some(_) = pdfium_or_skip() else { return };
    let (rendered, _worker) = render("letter-portrait.pdf", 0, 200);

    // User-space bottom-left quadrant displays at the bottom-left.
    assert_color(sample(&rendered.rgba, 0.25, 0.75), RED, "bottom-left");
    assert_color(sample(&rendered.rgba, 0.75, 0.25), WHITE, "top-right");
}

/// /Rotate 90 must rotate the render: the user-space bottom-left (red)
/// quadrant displays at the TOP-left of the landscape bitmap.
#[test]
fn rotate_90_moves_bottom_left_quadrant_to_display_top_left() {
    let Some(_) = pdfium_or_skip() else { return };
    let (rendered, _worker) = render("rotated-90.pdf", 0, 396);

    // Landscape render of the portrait page.
    assert_eq!(rendered.rgba.width(), 396);
    let expected_height = (396.0_f64 / 792.0 * 612.0).round() as u32;
    assert_eq!(rendered.rgba.height(), expected_height);

    assert_color(sample(&rendered.rgba, 0.25, 0.25), RED, "top-left");
    assert_color(sample(&rendered.rgba, 0.75, 0.75), WHITE, "bottom-right");
}

#[test]
fn landscape_green_quadrant_lands_top_right() {
    let Some(_) = pdfium_or_skip() else { return };
    let (rendered, _worker) = render("a4-landscape.pdf", 0, 421);

    assert_color(sample(&rendered.rgba, 0.75, 0.25), GREEN, "top-right");
    assert_color(sample(&rendered.rgba, 0.25, 0.75), WHITE, "bottom-left");
}

/// The rendered area is the crop box: blue fills the crop's bottom-left
/// quadrant, and geometry matches the 489.6×633.6 display size.
#[test]
fn cropped_page_renders_crop_box_area() {
    let Some(_) = pdfium_or_skip() else { return };
    let (rendered, _worker) = render("cropped.pdf", 0, 245);

    assert_eq!(rendered.rgba.width(), 245);
    let expected_height = (245.0_f64 / 489.6 * 633.6).round() as u32;
    assert_eq!(rendered.rgba.height(), expected_height);

    assert_color(sample(&rendered.rgba, 0.25, 0.75), BLUE, "bottom-left");
    assert_color(sample(&rendered.rgba, 0.75, 0.25), WHITE, "top-right");
}

#[test]
fn mixed_sizes_render_each_page() {
    let Some(_) = pdfium_or_skip() else { return };

    let worker = PdfWorker::spawn();
    let loaded = pollster::block_on(worker.load(fixture("mixed-sizes.pdf")))
        .expect("worker alive")
        .expect("fixture loads");

    let expected = [
        (RED, 0.25, 0.75),    // page 1: red bottom-left
        (GREEN, 0.75, 0.25),  // page 2: green top-right
        (YELLOW, 0.25, 0.25), // page 3: yellow top-left
    ];
    for (page_index, (color, fx, fy)) in expected.iter().enumerate() {
        let rendered =
            pollster::block_on(worker.render_page(loaded.handle, page_index as u32, 200))
                .expect("worker alive")
                .expect("render succeeds");
        assert_color(
            sample(&rendered.rgba, *fx, *fy),
            *color,
            "page {page_index}",
        );
    }

    worker.close(loaded.handle);
}

#[test]
fn render_of_unknown_document_reports_error() {
    let Some(_) = pdfium_or_skip() else { return };
    let worker = PdfWorker::spawn();
    let bogus = mark_pdf::PdfDocumentHandle::from_u64(9999);
    let error = pollster::block_on(worker.render_page(bogus, 0, 100))
        .expect("worker alive")
        .unwrap_err();
    assert!(matches!(error, RenderPageError::UnknownDocument));
}
