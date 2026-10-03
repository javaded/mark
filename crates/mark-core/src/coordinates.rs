//! PDF page geometry and coordinate mapping (plan.md §8.1).
//!
//! PDF user space has its origin at the page's bottom-left corner, with y
//! growing upward. Mark's document coordinates — the coordinates every
//! placed object uses — have their origin at the page's top-left corner,
//! with y growing downward, in the orientation the page is *displayed* in
//! (after applying the page's intrinsic `/Rotate` and cropping to the crop
//! box).
//!
//! [`PageGeometry`] captures the raw PDF page box; [`PageCoordinateMapper`]
//! converts points between the two spaces. Zoom and pan from screen space
//! live in [`crate::ViewTransform`], not here.

use crate::model::{PageRotation, Vec2};

/// The geometry of a PDF page in user space: crop box origin and size
/// (points), plus the page's intrinsic rotation.
///
/// `size` is the *unrotated* box; [`PageGeometry::display_size`] gives the
/// rotated size the viewer shows. `origin` is the bottom-left corner of the
/// page box in user-space coordinates (non-zero when the page uses a crop
/// box offset from the media box origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    pub origin: Vec2,
    pub size: Vec2,
    pub rotation: PageRotation,
}

impl PageGeometry {
    pub fn new(origin: Vec2, size: Vec2, rotation: PageRotation) -> Self {
        Self {
            origin,
            size,
            rotation,
        }
    }

    /// A geometry for a plain (uncropped, unrotated) page of the given size.
    pub fn unrotated(size: Vec2) -> Self {
        Self::new(Vec2::new(0.0, 0.0), size, PageRotation::None)
    }

    /// Size of the page as displayed, in points: width/height swap for
    /// quarter-turn rotations.
    pub fn display_size(&self) -> Vec2 {
        match self.rotation {
            PageRotation::Degrees90 | PageRotation::Degrees270 => {
                Vec2::new(self.size.y, self.size.x)
            }
            PageRotation::None | PageRotation::Degrees180 => self.size,
        }
    }

    pub fn mapper(&self) -> PageCoordinateMapper {
        PageCoordinateMapper(*self)
    }
}

/// Converts points between PDF user space (bottom-left origin, unrotated)
/// and Mark's document/display space (top-left origin, post-rotation).
///
/// All values are points (1/72 inch). The mapping is affine and total —
/// points outside the page convert too; clamping to the page is the
/// caller's policy decision, not the mapper's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageCoordinateMapper(PageGeometry);

impl PageCoordinateMapper {
    pub fn new(geometry: PageGeometry) -> Self {
        Self(geometry)
    }

    pub fn geometry(&self) -> &PageGeometry {
        &self.0
    }

    /// Size of the page as displayed, in points.
    pub fn display_size(&self) -> Vec2 {
        self.0.display_size()
    }

    /// Maps a PDF user-space point to display coordinates.
    pub fn user_to_display(&self, point: Vec2) -> Vec2 {
        let (x0, y0) = (self.0.origin.x, self.0.origin.y);
        let (w, h) = (self.0.size.x, self.0.size.y);
        match self.0.rotation {
            // Display bottom-left (0, h) is user bottom-left (x0, y0).
            PageRotation::None => Vec2::new(point.x - x0, y0 + h - point.y),
            // Rotated 90° clockwise: user bottom-left shows at display
            // top-left (0, 0).
            PageRotation::Degrees90 => Vec2::new(point.y - y0, point.x - x0),
            // Rotated 180°: user bottom-left shows at display top-right.
            PageRotation::Degrees180 => Vec2::new(x0 + w - point.x, point.y - y0),
            // Rotated 270° (counter-clockwise quarter turn): user bottom-left
            // shows at display bottom-right.
            PageRotation::Degrees270 => Vec2::new(y0 + h - point.y, x0 + w - point.x),
        }
    }

    /// Maps a display-space point to PDF user-space coordinates.
    pub fn display_to_user(&self, point: Vec2) -> Vec2 {
        let (x0, y0) = (self.0.origin.x, self.0.origin.y);
        let (w, h) = (self.0.size.x, self.0.size.y);
        match self.0.rotation {
            PageRotation::None => Vec2::new(x0 + point.x, y0 + h - point.y),
            PageRotation::Degrees90 => Vec2::new(x0 + point.y, y0 + point.x),
            PageRotation::Degrees180 => Vec2::new(x0 + w - point.x, y0 + point.y),
            PageRotation::Degrees270 => Vec2::new(x0 + w - point.y, y0 + h - point.x),
        }
    }
}
