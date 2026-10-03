//! Coordinate fixture suite (plan.md §8.1, §20.2).
//!
//! Covers the full mapping pipeline for portrait, landscape, rotated,
//! US Letter, and arbitrary page sizes, with crop-box offsets, zooms of
//! 50/100/300%, and panned views. The round-trip property —
//! `screen → document → screen` (and `user → display → user`) returns the
//! same point within float tolerance — must hold for every case.

use mark_core::{PageCoordinateMapper, PageGeometry, PageRotation, Vec2, ViewTransform};

const TOLERANCE: f32 = 1e-3;

/// US Letter portrait: 612×792 pt.
fn letter(rotation: PageRotation) -> PageGeometry {
    PageGeometry::new(Vec2::new(0.0, 0.0), Vec2::new(612.0, 792.0), rotation)
}

/// A4 landscape: 842×595 pt.
fn a4_landscape() -> PageGeometry {
    PageGeometry::unrotated(Vec2::new(842.0, 595.0))
}

/// Arbitrary size with a crop box offset from the media box origin.
fn cropped_arbitrary() -> PageGeometry {
    PageGeometry::new(
        Vec2::new(100.0, 50.0),
        Vec2::new(489.6, 633.6),
        PageRotation::None,
    )
}

/// The four user-space box corners: BL, BR, TR, TL.
fn user_corners(geometry: &PageGeometry) -> [Vec2; 4] {
    let (x0, y0) = (geometry.origin.x, geometry.origin.y);
    let (w, h) = (geometry.size.x, geometry.size.y);
    [
        Vec2::new(x0, y0),
        Vec2::new(x0 + w, y0),
        Vec2::new(x0 + w, y0 + h),
        Vec2::new(x0, y0 + h),
    ]
}

fn assert_close(actual: Vec2, expected: Vec2, context: &str) {
    assert!(
        (actual.x - expected.x).abs() < TOLERANCE && (actual.y - expected.y).abs() < TOLERANCE,
        "{context}: expected ({}, {}), got ({}, {})",
        expected.x,
        expected.y,
        actual.x,
        actual.y
    );
}

#[test]
fn display_size_swaps_for_quarter_turns_only() {
    assert_eq!(
        letter(PageRotation::None).display_size(),
        Vec2::new(612.0, 792.0)
    );
    assert_eq!(
        letter(PageRotation::Degrees180).display_size(),
        Vec2::new(612.0, 792.0)
    );
    assert_eq!(
        letter(PageRotation::Degrees90).display_size(),
        Vec2::new(792.0, 612.0)
    );
    assert_eq!(
        letter(PageRotation::Degrees270).display_size(),
        Vec2::new(792.0, 612.0)
    );
    assert_eq!(a4_landscape().display_size(), Vec2::new(842.0, 595.0));
}

/// Rotation 0: user bottom-left shows at display bottom-left.
#[test]
fn corners_map_exactly_without_rotation() {
    let mapper = PageCoordinateMapper::new(letter(PageRotation::None));
    let [bl, br, tr, tl] = user_corners(mapper.geometry());

    assert_close(mapper.user_to_display(bl), Vec2::new(0.0, 792.0), "BL");
    assert_close(mapper.user_to_display(br), Vec2::new(612.0, 792.0), "BR");
    assert_close(mapper.user_to_display(tr), Vec2::new(612.0, 0.0), "TR");
    assert_close(mapper.user_to_display(tl), Vec2::new(0.0, 0.0), "TL");
}

/// /Rotate 90 (clockwise): the user-space bottom edge becomes the display's
/// left edge, so user bottom-left shows at display top-left.
#[test]
fn corners_map_exactly_at_rotate_90() {
    let mapper = PageCoordinateMapper::new(letter(PageRotation::Degrees90));
    let [bl, br, tr, tl] = user_corners(mapper.geometry());

    assert_close(mapper.user_to_display(bl), Vec2::new(0.0, 0.0), "BL");
    assert_close(mapper.user_to_display(br), Vec2::new(0.0, 612.0), "BR");
    assert_close(mapper.user_to_display(tr), Vec2::new(792.0, 612.0), "TR");
    assert_close(mapper.user_to_display(tl), Vec2::new(792.0, 0.0), "TL");
}

/// /Rotate 180: user bottom-left shows at display top-right.
#[test]
fn corners_map_exactly_at_rotate_180() {
    let mapper = PageCoordinateMapper::new(letter(PageRotation::Degrees180));
    let [bl, br, tr, tl] = user_corners(mapper.geometry());

    assert_close(mapper.user_to_display(bl), Vec2::new(612.0, 0.0), "BL");
    assert_close(mapper.user_to_display(br), Vec2::new(0.0, 0.0), "BR");
    assert_close(mapper.user_to_display(tr), Vec2::new(0.0, 792.0), "TR");
    assert_close(mapper.user_to_display(tl), Vec2::new(612.0, 792.0), "TL");
}

/// /Rotate 270 (counter-clockwise quarter turn): user bottom-left shows at
/// display bottom-right.
#[test]
fn corners_map_exactly_at_rotate_270() {
    let mapper = PageCoordinateMapper::new(letter(PageRotation::Degrees270));
    let [bl, br, tr, tl] = user_corners(mapper.geometry());

    assert_close(mapper.user_to_display(bl), Vec2::new(792.0, 612.0), "BL");
    assert_close(mapper.user_to_display(br), Vec2::new(792.0, 0.0), "BR");
    assert_close(mapper.user_to_display(tr), Vec2::new(0.0, 0.0), "TR");
    assert_close(mapper.user_to_display(tl), Vec2::new(0.0, 612.0), "TL");
}

/// A landscape page (no rotation) maps like rotation 0, just wider.
#[test]
fn landscape_page_maps_with_plain_flip() {
    let mapper = PageCoordinateMapper::new(a4_landscape());

    // User BL (0,0) → display bottom-left.
    assert_close(
        mapper.user_to_display(Vec2::new(0.0, 0.0)),
        Vec2::new(0.0, 595.0),
        "BL",
    );
    // User TL (0,595) → display top-left.
    assert_close(
        mapper.user_to_display(Vec2::new(0.0, 595.0)),
        Vec2::new(0.0, 0.0),
        "TL",
    );
}

/// A crop box offset from the media box origin shifts the mapping; the
/// crop box's bottom-left corner is display (0, display_height).
#[test]
fn crop_box_origin_shifts_mapping() {
    let geometry = cropped_arbitrary();
    let mapper = PageCoordinateMapper::new(geometry);
    let [bl, br, tr, tl] = user_corners(&geometry);

    assert_close(mapper.user_to_display(bl), Vec2::new(0.0, 633.6), "BL");
    assert_close(mapper.user_to_display(tl), Vec2::new(0.0, 0.0), "TL");
    assert_close(mapper.user_to_display(tr), Vec2::new(489.6, 0.0), "TR");
    assert_close(mapper.user_to_display(br), Vec2::new(489.6, 633.6), "BR");

    // Media box corners outside the crop box map to negative coordinates —
    // affine and total, never clamped.
    let user_media_bl = Vec2::new(0.0, 0.0);
    let display = mapper.user_to_display(user_media_bl);
    assert!(display.x < 0.0 && display.y > 633.6, "media BL {display:?}");
}

/// Arbitrary page sizes at all rotations: user → display → user returns the
/// same point (plan.md §8.1 round-trip property).
#[test]
fn user_display_round_trips_at_all_rotations() {
    let sizes = [
        Vec2::new(612.0, 792.0),   // US Letter
        Vec2::new(842.0, 595.0),   // A4 landscape
        Vec2::new(500.0, 400.0),   // arbitrary
        Vec2::new(198.42, 305.97), // arbitrary fractional
    ];
    let origins = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)];
    let rotations = [
        PageRotation::None,
        PageRotation::Degrees90,
        PageRotation::Degrees180,
        PageRotation::Degrees270,
    ];

    for size in sizes {
        for origin in origins {
            for rotation in rotations {
                let mapper = PageCoordinateMapper::new(PageGeometry::new(origin, size, rotation));
                for point in [
                    Vec2::new(0.0, 0.0),
                    origin,
                    Vec2::new(origin.x + size.x / 2.0, origin.y + size.y / 3.0),
                    Vec2::new(origin.x + size.x, origin.y + size.y),
                    Vec2::new(-13.75, 4242.5),
                ] {
                    let display = mapper.user_to_display(point);
                    let round_trip = mapper.display_to_user(display);
                    assert_close(
                        round_trip,
                        point,
                        &format!("size {size:?} origin {origin:?} rot {rotation:?}"),
                    );
                }
            }
        }
    }
}

/// The full pipeline — PDF user space → display → zoom → pan → screen and
/// back — round-trips for zooms 50/100/300% and panned views (plan.md §8).
#[test]
fn full_pipeline_round_trips_with_zoom_and_pan() {
    let views = [
        ViewTransform::identity(),
        ViewTransform::new(0.5, 0.0, 0.0),
        ViewTransform::new(3.0, 0.0, 0.0),
        ViewTransform::new(1.0, 120.0, -40.0),
        ViewTransform::new(0.5, 250.5, -80.25),
        ViewTransform::new(3.0, -250.5, 80.25),
    ];
    let geometries = [
        letter(PageRotation::None),
        letter(PageRotation::Degrees90),
        letter(PageRotation::Degrees180),
        letter(PageRotation::Degrees270),
        a4_landscape(),
        cropped_arbitrary(),
    ];

    for geometry in geometries {
        let mapper = geometry.mapper();
        for view in views {
            for user in user_corners(&geometry) {
                let display = mapper.user_to_display(user);
                let (sx, sy) = view.document_to_screen(display.x, display.y);
                let (dx, dy) = view.screen_to_document(sx, sy);
                let round_trip = mapper.display_to_user(Vec2::new(dx, dy));
                assert_close(
                    round_trip,
                    user,
                    &format!("geometry {geometry:?} view {view:?}"),
                );
            }
        }
    }
}

/// Screen → document stays consistent end to end: a known screen point maps
/// to the expected user point at a given zoom/pan (spot check).
#[test]
fn screen_point_maps_to_expected_user_point() {
    // Letter, /Rotate 90: display is 792×612. Zoom 2×, pan (10, 20).
    let mapper = PageCoordinateMapper::new(letter(PageRotation::Degrees90));
    let view = ViewTransform::new(2.0, 10.0, 20.0);

    // Display (0,0) is user BL; at zoom 2 + pan (10,20) it sits at screen
    // (10, 20).
    let (dx, dy) = view.screen_to_document(10.0, 20.0);
    assert_eq!((dx, dy), (0.0, 0.0));
    assert_close(
        mapper.display_to_user(Vec2::new(dx, dy)),
        Vec2::new(0.0, 0.0),
        "user BL",
    );

    // Display center (396, 306) → user center (306, 396).
    let (dx, dy) = view.screen_to_document(802.0, 632.0);
    assert_close(Vec2::new(dx, dy), Vec2::new(396.0, 306.0), "display center");
    assert_close(
        mapper.display_to_user(Vec2::new(dx, dy)),
        Vec2::new(306.0, 396.0),
        "user center",
    );
}
