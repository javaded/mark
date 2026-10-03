//! The PDFium worker thread (plan.md §6.4): sequential request processing,
//! handle isolation between documents, close semantics.

mod common;

use common::{assert_color, fixture, pdfium_or_skip, sample};

#[test]
fn worker_serves_load_then_render_then_close() {
    let Some(_) = pdfium_or_skip() else { return };
    let worker = mark_pdf::PdfWorker::spawn();

    let first = pollster::block_on(worker.load(fixture("letter-portrait.pdf")))
        .expect("worker alive")
        .expect("load");
    let second = pollster::block_on(worker.load(fixture("a4-landscape.pdf")))
        .expect("worker alive")
        .expect("load");

    // Distinct handles per document.
    assert_ne!(first.handle, second.handle);

    // Both documents render through the same worker sequentially.
    for (loaded, width, height) in [(&first, 612.0_f32, 792.0_f32), (&second, 842.0, 595.0)] {
        let rendered = pollster::block_on(worker.render_page(loaded.handle, 0, 150))
            .expect("worker alive")
            .expect("render");
        let expected_height = (150.0 / width * height).round() as u32;
        assert_eq!(
            (rendered.rgba.width(), rendered.rgba.height()),
            (150, expected_height)
        );
    }

    // Closing releases just that document.
    worker.close(first.handle);
    use mark_pdf::RenderPageError;
    let error = pollster::block_on(worker.render_page(first.handle, 0, 150))
        .expect("worker alive")
        .unwrap_err();
    assert!(matches!(error, RenderPageError::UnknownDocument));
    // The other document still works.
    let rendered = pollster::block_on(worker.render_page(second.handle, 0, 100))
        .expect("worker alive")
        .expect("render after close of sibling");
    assert_eq!(rendered.rgba.width(), 100);
}

#[test]
fn sequential_requests_preserve_order_across_documents() {
    let Some(_) = pdfium_or_skip() else { return };
    let worker = mark_pdf::PdfWorker::spawn();

    // Fire renders at two documents without awaiting in between; the
    // worker must answer both correctly despite interleaving.
    let letter = pollster::block_on(worker.load(fixture("letter-portrait.pdf")))
        .expect("worker alive")
        .expect("load");
    let rotated = pollster::block_on(worker.load(fixture("rotated-90.pdf")))
        .expect("worker alive")
        .expect("load");

    let letter_render = worker.render_page(letter.handle, 0, 120);
    let rotated_render = worker.render_page(rotated.handle, 0, 120);

    let letter = pollster::block_on(letter_render)
        .expect("worker alive")
        .expect("render");
    let rotated = pollster::block_on(rotated_render)
        .expect("worker alive")
        .expect("render");

    // Portrait stays portrait; rotated-90 renders landscape.
    assert!(letter.rgba.height() > letter.rgba.width());
    assert!(rotated.rgba.width() > rotated.rgba.height());
    // Red quadrant: bottom-left for letter, top-left for rotated-90.
    assert_color(sample(&letter.rgba, 0.25, 0.75), [255, 0, 0], "letter");
    assert_color(sample(&rotated.rgba, 0.25, 0.25), [255, 0, 0], "rotated-90");
}
