//! Regenerates the PDF test fixtures in `resources/test-documents/`
//! (plan.md §20.3).
//!
//! Fixtures are hand-built minimal PDFs so MediaBox/CropBox/Rotate are
//! exactly what the tests assert. Each fixture paints one colored quadrant
//! in user space; render tests assert where that quadrant lands in display
//! space (proving rotation handling end to end). The text-heavy fixture
//! uses real (unembedded Helvetica) text objects so export tests can prove
//! text survives signing; the scanned fixture embeds a JPEG image page the
//! way scanned documents do.
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

    let text_font_object = b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec();
    let scanned_jpeg = scanned_jpeg();

    let fixtures: Vec<(&str, Vec<PageSpec>)> = vec![
        (
            "letter-portrait.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: None,
                rotate: 0,
                content: quadrant(Quadrant::BottomLeft, 612.0, 792.0, "1 0 0"), // red
                resources: String::new(),
                extra_objects: vec![],
            }],
        ),
        (
            "a4-landscape.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 842.0, 595.0],
                crop: None,
                rotate: 0,
                content: quadrant(Quadrant::TopRight, 842.0, 595.0, "0 1 0"), // green
                resources: String::new(),
                extra_objects: vec![],
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
                resources: String::new(),
                extra_objects: vec![],
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
                resources: String::new(),
                extra_objects: vec![],
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
                    resources: String::new(),
                    extra_objects: vec![],
                },
                PageSpec {
                    media: [0.0, 0.0, 792.0, 612.0],
                    crop: None,
                    rotate: 0,
                    content: quadrant(Quadrant::TopRight, 792.0, 612.0, "0 1 0"), // green
                    resources: String::new(),
                    extra_objects: vec![],
                },
                PageSpec {
                    media: [0.0, 0.0, 500.0, 400.0],
                    crop: None,
                    rotate: 0,
                    content: quadrant(Quadrant::TopLeft, 500.0, 400.0, "1 1 0"), // yellow
                    resources: String::new(),
                    extra_objects: vec![],
                },
            ],
        ),
        (
            "text-heavy.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: None,
                rotate: 0,
                content: text_lines(30),
                // The font object number is filled in during assembly.
                resources: "/Font<</F1 {font} 0 R>>".to_owned(),
                extra_objects: vec![text_font_object],
            }],
        ),
        (
            "scanned.pdf",
            vec![PageSpec {
                media: [0.0, 0.0, 612.0, 792.0],
                crop: None,
                rotate: 0,
                // The whole page is the embedded scan, drawn to fill it.
                content: "q 612 0 0 792 0 0 cm /Im0 Do Q".to_owned(),
                resources: "/XObject<</Im0 {font} 0 R>>".to_owned(),
                extra_objects: vec![jpeg_image_object(&scanned_jpeg, 122, 158)],
            }],
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

/// `count` lines of selectable Helvetica text, 24 pt apart from the top.
fn text_lines(count: usize) -> String {
    let mut content = String::new();
    for line in 0..count {
        let y = 720.0 - line as f32 * 24.0;
        content += &format!(
            "BT /F1 12 Tf 72 {y} Td (This is selectable test line {} of the text-heavy page. It must survive signing.) Tj ET\n",
            line + 1
        );
    }
    content
}

/// A synthetic scan: a 122×158 grayscale gradient with a white corner, as
/// JPEG bytes (the format scanned-document PDFs embed).
fn scanned_jpeg() -> Vec<u8> {
    let (width, height) = (122_u32, 158_u32);
    let image = image::GrayImage::from_fn(width, height, |x, y| {
        if x < width / 4 && y < height / 4 {
            image::Luma([240]) // white corner marker
        } else {
            // Vertical gradient from light to dark.
            image::Luma([(255 - (y * 200) / height) as u8])
        }
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .expect("encode scanned fixture jpeg");
    bytes
}

/// The image XObject object body wrapping DCTDecode (JPEG) bytes.
fn jpeg_image_object(jpeg: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut body = format!(
        "<</Type/XObject/Subtype/Image/Width {width}/Height {height}/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/DCTDecode/Length {}>>\nstream\n",
        jpeg.len()
    )
    .into_bytes();
    body.extend_from_slice(jpeg);
    body.extend_from_slice(b"\nendstream");
    body
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
    /// Extra `/Resources` dict entries; `{font}` resolves to the page's
    /// first extra object number.
    resources: String,
    /// Additional objects this page owns (fonts, image XObjects).
    extra_objects: Vec<Vec<u8>>,
}

/// Builds a minimal single-generation PDF with correct xref offsets.
fn build_pdf(pages: Vec<PageSpec>) -> Vec<u8> {
    // Object numbering: 1 catalog, 2 page tree, then per page its page
    // object, content stream, and extra objects.
    let mut next_object = 3;
    let mut numbers = Vec::with_capacity(pages.len());
    for page in &pages {
        let page_object = next_object;
        let content_object = page_object + 1;
        let extras: Vec<usize> = (0..page.extra_objects.len())
            .map(|i| content_object + 1 + i)
            .collect();
        numbers.push((page_object, content_object, extras));
        next_object = content_object + 1 + page.extra_objects.len();
    }

    let kids: Vec<String> = numbers
        .iter()
        .map(|(page, _, _)| format!("{page} 0 R"))
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

    for (i, spec) in pages.into_iter().enumerate() {
        let (page_obj, content_obj, ref extras) = numbers[i];
        let resources = if spec.resources.is_empty() {
            "/Resources<<>>".to_owned()
        } else {
            // `{font}` refers to the first extra object (the fixture's
            // single resource target).
            let resolved = if extras.is_empty() {
                spec.resources.clone()
            } else {
                spec.resources.replace("{font}", &extras[0].to_string())
            };
            format!("/Resources<<{resolved}>>")
        };
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
        dict += &resources;
        dict += &format!("/Contents {content_obj} 0 R>>");
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
        for (extra, number) in spec.extra_objects.into_iter().zip(extras.iter().copied()) {
            objects.push((number, extra));
        }
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
