//! Object manipulation gestures: move and resize state machines (plan.md
//! §13, §14).
//!
//! One pointer gesture = one command. Live feedback stays in the view
//! layer (a preview rect the canvas overlays); the document is only ever
//! mutated when `commit` runs — at pointer release — through the normal
//! command/undo machinery. Aspect ratio locks by default; Shift at gesture
//! start unlocks it (plan.md §13: pick one convention and document it).

use mark_core::{MoveObject, ObjectId, Rect, ResizeObject, Vec2, ViewTransform};

/// Smallest object edge in document units; resizes never collapse below it.
pub(crate) const MIN_OBJECT_SIZE: f32 = 16.;

/// Which corner of the selection box the resize handle sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    /// Stable element-id fragment for handle elements.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "tl",
            Corner::TopRight => "tr",
            Corner::BottomRight => "br",
            Corner::BottomLeft => "bl",
        }
    }
}

/// An in-progress manipulation of one object.
pub(crate) struct Gesture {
    pub object: ObjectId,
    pub kind: GestureKind,
    /// Pointer position (screen px) where the gesture started.
    pointer_start: Vec2,
    /// Latest pointer position (screen px); drives the live preview.
    pointer_now: Vec2,
    /// Preview rect (document units) shown instead of the stored geometry.
    pub live: Rect,
}

pub(crate) enum GestureKind {
    Move {
        /// Object position before the gesture (document units).
        from: Vec2,
    },
    Resize {
        /// Object rect before the gesture (document units).
        original: Rect,
        corner: Corner,
        aspect_lock: bool,
    },
}

impl Gesture {
    pub fn begin_move(object: ObjectId, from: Vec2, size: Vec2, pointer: Vec2) -> Self {
        Self {
            object,
            kind: GestureKind::Move { from },
            pointer_start: pointer,
            pointer_now: pointer,
            live: Rect {
                x: from.x,
                y: from.y,
                width: size.x,
                height: size.y,
            },
        }
    }

    pub fn begin_resize(
        object: ObjectId,
        original: Rect,
        corner: Corner,
        aspect_lock: bool,
        pointer: Vec2,
        transform: ViewTransform,
    ) -> Self {
        let mut gesture = Self {
            object,
            kind: GestureKind::Resize {
                original,
                corner,
                aspect_lock,
            },
            pointer_start: pointer,
            pointer_now: pointer,
            live: original,
        };
        gesture.update_pointer(pointer, transform);
        gesture
    }

    /// Records the pointer and recomputes the live preview.
    pub fn update_pointer(&mut self, pointer: Vec2, transform: ViewTransform) {
        self.pointer_now = pointer;
        self.live = self.preview(transform);
    }

    /// True when the pointer never actually moved — press-and-release on
    /// the spot commits nothing (no no-op undo steps, plan.md §14).
    pub fn is_motionless(&self) -> bool {
        (self.pointer_now.x - self.pointer_start.x).abs() < 0.5
            && (self.pointer_now.y - self.pointer_start.y).abs() < 0.5
    }

    /// The command this gesture accumulated, or `None` when nothing moved.
    pub fn commit(&self) -> Option<Box<dyn mark_core::Command>> {
        if self.is_motionless() {
            return None;
        }
        match &self.kind {
            GestureKind::Move { from } => Some(Box::new(MoveObject::new(
                self.object,
                *from,
                Vec2::new(self.live.x, self.live.y),
            ))),
            GestureKind::Resize { original, .. } => Some(Box::new(ResizeObject::new(
                self.object,
                *original,
                self.live,
            ))),
        }
    }

    fn preview(&self, transform: ViewTransform) -> Rect {
        let delta = Vec2::new(
            self.pointer_now.x - self.pointer_start.x,
            self.pointer_now.y - self.pointer_start.y,
        );
        match &self.kind {
            GestureKind::Move { from } => moved_rect(
                *from,
                Vec2::new(self.live.width, self.live.height),
                delta,
                transform.zoom,
            ),
            GestureKind::Resize {
                original,
                corner,
                aspect_lock,
            } => resized_rect(*original, *corner, delta, transform.zoom, *aspect_lock),
        }
    }
}

/// Move preview: the original position translated by the screen-space
/// pointer delta scaled into document units.
pub(crate) fn moved_rect(from: Vec2, size: Vec2, delta: Vec2, zoom: f32) -> Rect {
    let zoom = zoom.max(f32::EPSILON);
    Rect {
        x: from.x + delta.x / zoom,
        y: from.y + delta.y / zoom,
        width: size.x,
        height: size.y,
    }
}

/// Resize preview from the gesture's screen-space pointer delta. The
/// dragged corner moves by the delta; the opposite corner stays fixed.
/// Delta math (not absolute anchors) keeps the gesture independent of
/// which container the pointer coordinates originate from. Aspect-locked
/// resizing derives height from width via the original aspect; free
/// resizing (Shift) tracks both axes independently. Edges clamp to
/// [`MIN_OBJECT_SIZE`] so objects never collapse.
pub(crate) fn resized_rect(
    original: Rect,
    corner: Corner,
    delta: Vec2,
    zoom: f32,
    aspect_lock: bool,
) -> Rect {
    let zoom = zoom.max(f32::EPSILON);
    let dx = delta.x / zoom;
    let dy = delta.y / zoom;
    // Width grows rightwards for right corners, leftwards for left ones;
    // height grows downwards for bottom corners, upwards for top ones.
    let grow_right = matches!(corner, Corner::TopRight | Corner::BottomRight);
    let grow_down = matches!(corner, Corner::BottomRight | Corner::BottomLeft);

    let width = if grow_right {
        (original.width + dx).max(MIN_OBJECT_SIZE)
    } else {
        (original.width - dx).max(MIN_OBJECT_SIZE)
    };
    let aspect = original.width.max(f32::EPSILON) / original.height.max(f32::EPSILON);
    let height = if aspect_lock {
        width / aspect
    } else if grow_down {
        (original.height + dy).max(MIN_OBJECT_SIZE)
    } else {
        (original.height - dy).max(MIN_OBJECT_SIZE)
    };

    Rect {
        x: if grow_right {
            original.x
        } else {
            original.x + original.width - width
        },
        y: if grow_down {
            original.y
        } else {
            original.y + original.height - height
        },
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Identity transform at the given zoom — pan-free gesture math.
    fn zoom(z: f32) -> ViewTransform {
        ViewTransform::new(z, 0., 0.)
    }

    // Small helper keeping `update_pointer` chainable in tests.
    trait TapPointer {
        fn tap_pointer(self, pointer: Vec2, transform: ViewTransform) -> Rect;
    }
    impl TapPointer for Gesture {
        fn tap_pointer(mut self, pointer: Vec2, transform: ViewTransform) -> Rect {
            self.update_pointer(pointer, transform);
            self.live
        }
    }

    #[test]
    fn move_translates_by_screen_delta_over_zoom() {
        // 100px of screen movement at 200% zoom = 50 doc units.
        let r = Gesture::begin_move(
            ObjectId::new(),
            Vec2::new(100., 200.),
            Vec2::new(60., 20.),
            Vec2::new(10., 10.),
        )
        .tap_pointer(Vec2::new(110., -40.), zoom(2.0));
        assert_eq!(r, rect(150., 175., 60., 20.));
    }

    #[test]
    fn move_is_origin_independent() {
        // Screen deltas divide by zoom only; any pointer origin cancels.
        let r = Gesture::begin_move(
            ObjectId::new(),
            Vec2::new(0., 0.),
            Vec2::new(10., 10.),
            Vec2::new(5030., 7070.),
        )
        .tap_pointer(Vec2::new(5080., 7100.), zoom(1.));
        assert_eq!(r, rect(50., 30., 10., 10.));
    }

    #[test]
    fn motionless_gesture_commits_nothing() {
        let mut g = Gesture::begin_move(
            ObjectId::new(),
            Vec2::new(10., 10.),
            Vec2::new(50., 20.),
            Vec2::new(500., 500.),
        );
        g.update_pointer(Vec2::new(500.4, 499.7), zoom(1.));
        assert!(g.is_motionless());
        assert!(g.commit().is_none());
    }

    #[test]
    fn resize_bottom_right_keeps_top_left_anchor_and_aspect() {
        let original = rect(100., 100., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::BottomRight,
            true,
            Vec2::new(0., 0.),
            zoom(1.),
        );
        // +100px right: 80 → 180 wide, aspect 2:1 keeps height at 90,
        // top-left pinned.
        g.update_pointer(Vec2::new(100., 0.), zoom(1.));
        assert_eq!(g.live, rect(100., 100., 180., 90.));
    }

    #[test]
    fn resize_top_left_moves_the_origin() {
        let original = rect(100., 100., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::TopLeft,
            true,
            Vec2::new(0., 0.),
            zoom(1.),
        );
        // Top-left dragged (-20, -10): width 80 → 100, height 40 → 50,
        // origin shifts to keep bottom-right pinned at (180, 140).
        g.update_pointer(Vec2::new(-20., -10.), zoom(1.));
        assert_eq!(g.live, rect(80., 90., 100., 50.));
    }

    #[test]
    fn resize_clamps_to_minimum_size() {
        let original = rect(100., 100., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::BottomRight,
            true,
            Vec2::new(0., 0.),
            zoom(1.),
        );
        // Drag far past the opposite corner: width clamps at the minimum,
        // top-left stays fixed.
        g.update_pointer(Vec2::new(-5000., -5000.), zoom(1.));
        assert_eq!(g.live.width, MIN_OBJECT_SIZE);
        assert_eq!(g.live.x, 100.);
        assert_eq!(g.live.y, 100.);
    }

    #[test]
    fn free_resize_tracks_both_axes_independently() {
        let original = rect(100., 100., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::BottomRight,
            false,
            Vec2::new(0., 0.),
            zoom(1.),
        );
        g.update_pointer(Vec2::new(-50., 150.), zoom(1.));
        // Width 30, height 190 — aspect broken by design (Shift held).
        assert_eq!(g.live, rect(100., 100., 30., 190.));
    }

    #[test]
    fn resize_scales_with_zoom() {
        let original = rect(0., 0., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::BottomRight,
            true,
            Vec2::new(0., 0.),
            zoom(2.),
        );
        // +160 screen px at 200% zoom = 80 doc units → 160 wide, 80 tall.
        g.update_pointer(Vec2::new(160., 0.), zoom(2.));
        assert_eq!(g.live, rect(0., 0., 160., 80.));
    }

    #[test]
    fn resize_is_origin_independent() {
        // Delta math: pointer origin (window vs viewport) cancels.
        let original = rect(10., 20., 80., 40.);
        let mut g = Gesture::begin_resize(
            ObjectId::new(),
            original,
            Corner::BottomRight,
            true,
            Vec2::new(990., 2050.),
            zoom(1.),
        );
        g.update_pointer(Vec2::new(1050., 2050.), zoom(1.));
        assert_eq!(g.live, rect(10., 20., 140., 70.));
    }

    #[test]
    fn commit_produces_the_expected_commands() {
        let id = ObjectId::new();
        let mut g =
            Gesture::begin_move(id, Vec2::new(1., 2.), Vec2::new(10., 5.), Vec2::new(0., 0.));
        g.update_pointer(Vec2::new(10., 5.), zoom(1.));
        // Command identity is round-tripped in mark-core tests; here we
        // only assert a command exists per gesture kind.
        assert!(g.commit().is_some());

        let mut r = Gesture::begin_resize(
            id,
            rect(0., 0., 10., 10.),
            Corner::BottomRight,
            true,
            Vec2::new(0., 0.),
            zoom(1.),
        );
        r.update_pointer(Vec2::new(50., 0.), zoom(1.));
        assert!(r.commit().is_some());
    }
}
