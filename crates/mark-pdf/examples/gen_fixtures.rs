//! Regenerates the PDF test fixtures in `resources/test-documents/`
//! (plan.md §20.3).
//!
//! Fixtures are hand-built minimal PDFs so MediaBox/CropBox/Rotate are
//! exactly what the tests assert. Each fixture paints one colored quadrant
//! in user space; render tests assert where that quadrant lands in display
//! space (proving rotation handling end to end).
//!
//! Run from the workspace root: `cargo run -p mark-pdf --example gen_fixtures`

use std::env;
use std::path::Path;

fn main() {
    let out_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "resources/test-documents".to_owned());
    let dir = Path::new(&out_dir);
    std::fs::create_dir_all(dir).expect("create fixture dir");

    let fixtures: &[(&str, Vec<PageSpec>)] = &[
        (
            "letter-portrait.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: None,
                rotate: 0,
                content: quadrant(Quadrant::BottomLeft, 612.0, 792.0, "1 0 0"), // red
            }],
        ),
        (
            "a4-landscape.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 842.0, 595.0],
                crop: None,
                rotate: 0,
                content: quadrant(Quadrant::TopRight, 842.0, 595.0, "0 1 0"), // green
            }],
        ),
        (
            "rotated-90.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: None,
                rotate: 90,
                // User-space bottom-left quadrant; /Rotate 90 displays it
                // at the top-left of a 792×612 view.
                content: quadrant(Quadrant::BottomLeft, 612.0, 792.0, "1 0 0"), // red
            }],
        ),
        (
            "cropped.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: Some([61.2, 79.2, 550.8, 712.8]),
                rotate: 0,
                // Quadrant of the crop box (489.6×633.6), user-space
                // bottom-left of the crop area.
                content: rect(61.2, 79.2, 244.8, 316.8, "0 0 1"), // blue
            }],
        ),
        (
            "mixed-sizes.pdf",
            vec![
                PageSpec {
                    media: [0.0, 0.0, 612.0, 792.0],
                    crop: None,
                    rotate: 0,
                    content: quadrant(Quadrant::BottomLeft, 612.0, 792.0, "1 0 0"), // red
                },
                PageSpec {
                    media: [0.0, 0.0, 792.0, 612.0],
                    crop: None,
                    rotate: 0,
                    content: quadrant(Quadrant::TopRight, 792.0, 612.0, "0 1 0"), // green
                },
                PageSpec {
                    media: [0.0, 0.0, 500.0, 400.0],
                    crop: None,
                    rotate: 0,
                    content: quadrant(Quadrant::TopLeft, 500.0, 400.0, "1 1 0"), // yellow
                },
            ],
        ),
    ];

    for (name, pages) in fixtures {
        let path = dir.join(name);
        std::fs::write(&path, build_pdf(pages)).expect("write fixture");
        println!("wrote {}", path.display());
    }
}

/// Which corner quadrant of the page (in user space) to paint.
enum Quadrant {
    BottomLeft,
    TopLeft,
    TopRight,
}

/// Content stream painting one colored quarter of a `width`×`height` page.
fn quadrant(which: Quadrant, width: f32, height: f32, rgb: &str) -> String {
    let (x, y) = match which {
        Quadrant::BottomLeft => (0.0, 0.0),
        Quadrant::TopLeft => (0.0, height / 2.0),
        Quadrant::TopRight => (width / 2.0, height / 2.0),
    };
    rect(x, y, width / 2.0, height / 2.0, rgb)
}

fn rect(x: f32, y: f32, w: f32, h: f32, rgb: &str) -> String {
    format!("{rgb} rg {x} {y} {w} {h} re f")
}

struct PageSpec {
    /// MediaBox [left bottom right top] in user-space points.
    media: [f32; 4],
    /// CropBox, when the page has one.
    crop: Option<[f32; 4]>,
    /// `/Rotate` in clockwise degrees.
    rotate: u16,
    /// Page content stream.
    content: String,
}

/// Builds a minimal single-generation PDF with correct xref offsets.
fn build_pdf(pages: &[PageSpec]) -> Vec<u8> {
    // Object numbering: 1 catalog, 2 page tree, then page/content pairs.
    let page_objs: Vec<(usize, usize)> = pages
        .iter()
        .enumerate()
        .map(|(i, _)| (3 + i * 2, 4 + i * 2))
        .collect();

    let kids: Vec<String> = page_objs
        .iter()
        .map(|(page, _)| format!("{page} 0 R"))
        .collect();

    let mut objects: Vec<(usize, Vec<u8>)> = Vec::new();
    objects.push((1, b"<</Type/Catalog/Pages 2 0 R>>".to_vec()));
    objects.push((
        2,
        format!(
            "<</Type/Pages/Kids[{}]/Count {}>>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    ));

    for (i, spec) in pages.iter().enumerate() {
        let (page_obj, content_obj) = page_objs[i];
        let mut dict = format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[{} {} {} {}]",
            spec.media[0], spec.media[1], spec.media[2], spec.media[3]
        );
        if let Some(crop) = spec.crop {
            dict += &format!("/CropBox[{} {} {} {}]", crop[0], crop[1], crop[2], crop[3]);
        }
        if spec.rotate != 0 {
            dict += &format!("/Rotate {}", spec.rotate);
        }
        dict += &format!("/Resources<<>>/Contents {content_obj} 0 R>>");
        objects.push((page_obj, dict.into_bytes()));
        objects.push((
            content_obj,
            format!(
                "<</Length {}>>\nstream\n{}\nendstream",
                spec.content.len(),
                spec.content
            )
            .into_bytes(),
        ));
    }

    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let count = objects.len() + 1;
    let mut offsets = vec![0_usize; count];

    for (num, body) in objects {
        offsets[num] = out.len();
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {count}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets[1..] {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<</Size {count}/Root 1 0 R>>\nstartxref\n{xref_at}\n%%EOF\n").as_bytes(),
    );
    out
}
