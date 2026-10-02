//! Raster image document engine for Mark.
//!
//! Layer C of the architecture (plan.md §3): an opened image becomes a
//! one-page document (plan.md §9.2). Decoding normalizes every format to
//! RGBA; the UI converts that to its render surface, and future export
//! embeds the RGBA data.

use std::path::Path;

use image::RgbaImage;
use mark_core::{Document, DocumentSource, Page};

/// An image opened as a one-page document: the domain document plus the
/// decoded RGBA page background the renderer paints.
#[derive(Debug)]
pub struct ImageDocument {
    document: Document,
    rgba: RgbaImage,
}

impl ImageDocument {
    /// Decodes the file at `path` and wraps it as a one-page document.
    ///
    /// Page size maps 1 pixel to 1 logical unit (1 inch = 72 points,
    /// plan.md §8) — a 96×96 image page is exactly 96×96 pt.
    pub fn load(path: &Path) -> Result<Self, LoadImageError> {
        let rgba = image::ImageReader::open(path)?.decode()?.to_rgba8();
        let (width, height) = rgba.dimensions();
        let page = Page::new(width as f32, height as f32);
        let document = Document::new(
            DocumentSource::Image {
                path: path.to_path_buf(),
            },
            vec![page],
        );
        Ok(Self { document, rgba })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn rgba(&self) -> &RgbaImage {
        &self.rgba
    }

    pub fn into_parts(self) -> (Document, RgbaImage) {
        (self.document, self.rgba)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadImageError {
    #[error("the file could not be read")]
    Io(#[from] std::io::Error),
    #[error("the image could not be decoded")]
    Decode(#[from] image::ImageError),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use image::{Rgba, RgbaImage};

    use super::*;

    enum Format {
        Png,
        Jpeg,
        WebP,
    }

    impl Format {
        fn name(&self) -> &'static str {
            match self {
                Format::Png => "png",
                Format::Jpeg => "jpeg",
                Format::WebP => "webp",
            }
        }

        fn extension(&self) -> &'static str {
            match self {
                Format::Png => "png",
                Format::Jpeg => "jpg",
                Format::WebP => "webp",
            }
        }
    }

    fn fixture(format: Format) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "mark-image-{}-{}.{}",
            format.name(),
            std::process::id(),
            format.extension()
        ));
        let image = RgbaImage::from_pixel(4, 3, Rgba([200, 40, 90, 255]));
        image::DynamicImage::ImageRgba8(image)
            .save(&path)
            .expect("fixture write");
        path
    }

    #[test]
    fn png_loads_as_one_page_with_pixel_sized_page() {
        let path = fixture(Format::Png);
        let loaded = ImageDocument::load(&path).unwrap();

        assert_eq!(loaded.document().page_count(), 1);
        let source_path = loaded.document().source().path();
        assert_eq!(source_path, path.as_path());

        let page = &loaded.document().pages()[0];
        assert_eq!((page.width(), page.height()), (4.0, 3.0));

        assert_eq!(loaded.rgba().dimensions(), (4, 3));
        assert_eq!(loaded.rgba().get_pixel(0, 0), &Rgba([200, 40, 90, 255]));
    }

    #[test]
    fn jpeg_loads_with_normalized_rgba() {
        let path = fixture(Format::Jpeg);
        let loaded = ImageDocument::load(&path).unwrap();

        assert_eq!(loaded.document().page_count(), 1);
        assert_eq!(
            (
                loaded.document().pages()[0].width(),
                loaded.document().pages()[0].height()
            ),
            (4.0, 3.0)
        );
        // JPEG is lossy: exact color is not asserted, only geometry and
        // the RGBA normalization (alpha = 255 everywhere).
        assert_eq!(loaded.rgba().dimensions(), (4, 3));
        for pixel in loaded.rgba().pixels() {
            assert_eq!(pixel.0[3], 255);
        }
    }

    #[test]
    fn webp_loads_as_one_page() {
        let path = fixture(Format::WebP);
        let loaded = ImageDocument::load(&path).unwrap();

        assert_eq!(loaded.document().page_count(), 1);
        assert_eq!(
            (
                loaded.document().pages()[0].width(),
                loaded.document().pages()[0].height()
            ),
            (4.0, 3.0)
        );
        assert_eq!(loaded.rgba().dimensions(), (4, 3));
    }

    #[test]
    fn missing_file_reports_io_error() {
        let error = ImageDocument::load(Path::new("/nonexistent/mark-test.png")).unwrap_err();
        assert!(matches!(error, LoadImageError::Io(_)));
    }

    #[test]
    fn non_image_file_reports_decode_error() {
        let mut path = std::env::temp_dir();
        path.push(format!("mark-image-notanimage-{}.txt", std::process::id()));
        std::fs::write(&path, "definitely not an image").unwrap();

        let error = ImageDocument::load(&path).unwrap_err();
        assert!(matches!(error, LoadImageError::Decode(_)));
    }
}
