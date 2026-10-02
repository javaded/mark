//! View transform: the single place screen/document coordinates meet
//! (plan.md §8).
//!
//! Document coordinates are logical page units (1 inch = 72 points). Screen
//! coordinates are pixels. The transform is zoom then pan:
//!
//! ```text
//! screen = document * zoom + pan
//! document = (screen - pan) / zoom
//! ```

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl ViewTransform {
    pub fn identity() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }

    pub fn new(zoom: f32, pan_x: f32, pan_y: f32) -> Self {
        Self { zoom, pan_x, pan_y }
    }

    pub fn document_to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.zoom + self.pan_x, y * self.zoom + self.pan_y)
    }

    pub fn screen_to_document(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pan_x) / self.zoom, (y - self.pan_y) / self.zoom)
    }

    /// Scale a length (width, height, corner radius) from document to screen.
    pub fn scale_to_screen(&self, length: f32) -> f32 {
        length * self.zoom
    }

    /// Scale a length from screen to document.
    pub fn scale_to_document(&self, length: f32) -> f32 {
        length / self.zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// screen → document → screen must return the same point (plan.md §52).
    #[test]
    fn round_trips_at_various_zooms_and_pans() {
        let cases = [
            ViewTransform::identity(),
            ViewTransform::new(0.5, 0.0, 0.0),
            ViewTransform::new(1.0, 120.0, -40.0),
            ViewTransform::new(3.0, -250.5, 80.25),
            ViewTransform::new(0.25, 1000.0, 2000.0),
        ];
        for transform in cases {
            for (x, y) in [(0.0, 0.0), (612.0, 792.0), (-13.75, 4242.5)] {
                let (sx, sy) = transform.document_to_screen(x, y);
                let (dx, dy) = transform.screen_to_document(sx, sy);
                assert!((dx - x).abs() < 1e-3, "x mismatch at {transform:?}");
                assert!((dy - y).abs() < 1e-3, "y mismatch at {transform:?}");
            }
        }
    }

    #[test]
    fn identity_maps_directly() {
        let t = ViewTransform::identity();
        assert_eq!(t.document_to_screen(100.0, 200.0), (100.0, 200.0));
        assert_eq!(t.screen_to_document(100.0, 200.0), (100.0, 200.0));
    }

    #[test]
    fn lengths_scale_with_zoom_only() {
        let t = ViewTransform::new(2.0, 500.0, -500.0);
        assert_eq!(t.scale_to_screen(72.0), 144.0);
        assert_eq!(t.scale_to_document(144.0), 72.0);
    }
}
