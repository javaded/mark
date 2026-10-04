//! Document canvas: the current page drawn at the viewer's zoom and pan
//! inside a viewport-sized stage (plan.md §10, §11).
//!
//! A `canvas` element probe measures the stage each frame; the resulting
//! viewport drives fit zoom and render sizing (§11). Wheel and drag pan the
//! page; the clamped view transform keeps the page reachable.

use std::sync::Arc;

use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, Empty, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels, Point,
    Render, RenderImage, ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement as _,
    Styled as _, TestSupportExt as _, WeakEntity, Window, canvas, div, img, px, rems,
};
use gpui_omarchy::Theme;
use mark_core::{ObjectId, Vec2, ViewTransform};

use crate::MarkApp;
use crate::manipulation::Corner;
use crate::viewer::ViewerState;

/// Drag value marking an active canvas pan (GPUI drag protocol).
struct PanCanvas;

impl Render for PanCanvas {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Drag value marking an object move gesture.
struct MoveObjectDrag;

impl Render for MoveObjectDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Drag value marking a handle resize gesture.
struct ResizeObjectDrag;

impl Render for ResizeObjectDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Corner handle side in screen px (fixed UI size, not zoom-scaled).
const HANDLE_SIZE: f32 = 10.;

/// A placed image object resolved for drawing: page-space placement plus
/// the cached asset bitmap (absent until its decode lands).
pub(crate) struct PlacedObject {
    pub id: ObjectId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
    pub image: Option<Arc<RenderImage>>,
    pub selected: bool,
}

/// Converts decoded RGBA pixels into GPUI's BGRA render surface.
///
/// GPUI samples render images as BGRA (gpui `RenderImage` contract); the
/// `image` crate decodes RGBA, so channels 0 and 2 swap per pixel.
pub(crate) fn render_image(rgba: &image::RgbaImage) -> RenderImage {
    let mut bgra = rgba.clone();
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    RenderImage::new([image::Frame::new(bgra)])
}

/// The document stage: viewport probe, the page bitmap positioned by the
/// view transform, the objects placed on it, the pan interactions, and the
/// floating selection toolbar (§13).
#[allow(clippy::too_many_arguments)]
pub(crate) fn stage(
    theme: &Theme,
    weak: &WeakEntity<MarkApp>,
    viewer: &mut ViewerState,
    placed: &[PlacedObject],
    toolbar: Option<AnyElement>,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    let page = viewer.current_page();
    let page_size = viewer.page_size(page);
    let transform = viewer.transform();
    let image = viewer.image_for(page);
    let probe_weak = weak.clone();

    div()
        .id("mark-canvas")
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .relative()
        .bg(theme.inset)
        .p(rems(1.5))
        .child(
            div()
                .id("mark-canvas-viewport")
                .test_support()
                .flex()
                .size_full()
                .overflow_hidden()
                .relative()
                // Viewport probe: the measured stage drives fit zoom and
                // render widths; refreshed requests are deferred out of
                // prepaint (async work never starts mid-frame).
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            let scale_factor = window.scale_factor();
                            let weak = probe_weak.clone();
                            let changed = weak
                                .update(cx, |app, cx| {
                                    let changed = app.viewer_mut().set_viewport(
                                        f32::from(bounds.size.width),
                                        f32::from(bounds.size.height),
                                        scale_factor,
                                    );
                                    if changed {
                                        cx.notify();
                                    }
                                    changed
                                })
                                .unwrap_or(false);
                            if changed {
                                cx.defer(move |cx| {
                                    weak.update(cx, |app, cx| app.refresh_view(cx)).ok();
                                });
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(stage_element(
                    theme,
                    transform,
                    page_size,
                    image.as_ref(),
                    placed,
                    weak,
                ))
                // Click-away: empty canvas clears the selection; object
                // handlers stop propagation first.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.clear_selection_if_idle(window, cx);
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    let (dx, dy) = scroll_delta(event);
                    this.viewer_mut().pan_by(dx, dy);
                    cx.notify();
                }))
                .on_drag(PanCanvas, {
                    let weak = weak.clone();
                    move |_: &PanCanvas, position: Point<Pixels>, _, cx| {
                        cx.stop_propagation();
                        weak.update(cx, |app, _| {
                            app.viewer_mut().pan_drag_started(Vec2::new(
                                f32::from(position.x),
                                f32::from(position.y),
                            ));
                        })
                        .ok();
                        cx.new(|_| PanCanvas)
                    }
                })
                .on_drag_move(
                    cx.listener(|this, event: &DragMoveEvent<PanCanvas>, _, cx| {
                        this.viewer_mut().pan_drag_moved(Vec2::new(
                            f32::from(event.event.position.x),
                            f32::from(event.event.position.y),
                        ));
                        cx.notify();
                    }),
                )
                .on_drag_move(
                    cx.listener(|this, event: &DragMoveEvent<MoveObjectDrag>, _, cx| {
                        this.move_gesture(event_pointer(event), cx);
                    }),
                )
                .on_drag_move(cx.listener(
                    |this, event: &DragMoveEvent<ResizeObjectDrag>, _, cx| {
                        this.move_gesture(event_pointer(event), cx);
                    },
                ))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.viewer_mut().pan_drag_ended();
                        this.end_gesture(cx);
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.viewer_mut().pan_drag_ended();
                        this.end_gesture(cx);
                    }),
                ),
        )
        // The contextual toolbar floats above the viewport: its presses
        // never reach the canvas below, so the selection survives.
        .children(toolbar)
}

/// The stage content: the positioned page with its placed objects, or the
/// centered rendering notice while no bitmap exists. The page bitmap
/// scales to the page box regardless of its pixel resolution (§11.1
/// reuse); objects map through the same view transform.
fn stage_element(
    theme: &Theme,
    transform: Option<ViewTransform>,
    page_size: Option<Vec2>,
    image: Option<&Arc<RenderImage>>,
    placed: &[PlacedObject],
    weak: &WeakEntity<MarkApp>,
) -> AnyElement {
    match (transform, page_size, image) {
        (Some(transform), Some(page), Some(image)) => {
            let (x, y) = transform.document_to_screen(0., 0.);
            div()
                .id("mark-canvas-page")
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(page.x * transform.zoom))
                .h(px(page.y * transform.zoom))
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .child(img(image.clone()).size_full())
                .children(
                    placed
                        .iter()
                        .map(|object| object_element(theme, &transform, object, weak)),
                )
                .into_any_element()
        }
        _ => div()
            .id("mark-canvas-notice")
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(rems(0.875))
                    .text_color(theme.secondary)
                    .child("Rendering page…"),
            )
            .into_any_element(),
    }
}

/// One placed object drawn over the page: the asset bitmap at its
/// page-space rect through the view transform. Pointer-down selects;
/// dragging moves; the selected object gains the accent outline and four
/// corner resize handles (plan.md §13).
///
/// The object is absolutely positioned inside the (positioned) page div,
/// so `left`/`top` are page-local: document units scaled by zoom, without
/// pan — the page itself already carries the pan offset.
fn object_element(
    theme: &Theme,
    transform: &ViewTransform,
    object: &PlacedObject,
    weak: &WeakEntity<MarkApp>,
) -> gpui_kit::AnyElement {
    let left = object.x * transform.zoom;
    let top = object.y * transform.zoom;
    let screen_w = object.width * transform.zoom;
    let screen_h = object.height * transform.zoom;
    let id = object.id;

    let select_weak = weak.clone();
    let drag_weak = weak.clone();
    let mut element = div()
        .id(format!("mark-object-{id}"))
        .test_support()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(screen_w))
        .h(px(screen_h))
        .opacity(object.opacity)
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            select_weak
                .update(cx, |app, cx| {
                    app.object_press(id, pointer_vec(event.position), cx);
                })
                .ok();
        })
        .on_drag(
            MoveObjectDrag,
            move |_: &MoveObjectDrag, position, _, cx| {
                cx.stop_propagation();
                drag_weak
                    .update(cx, |app, _| {
                        app.begin_move(id, pointer_vec(position));
                    })
                    .ok();
                cx.new(|_| MoveObjectDrag)
            },
        );
    element = match &object.image {
        Some(image) => element.child(img(image.clone()).size_full()),
        // Bitmap still decoding: a dashed placeholder keeps the placement
        // and selection visible.
        None => element
            .border_1()
            .border_dashed()
            .border_color(theme.border),
    };
    if object.selected {
        element = element.border_1().border_color(theme.accent).rounded_sm();
        for corner in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomRight,
            Corner::BottomLeft,
        ] {
            element = element.child(handle_element(theme, corner, id, screen_w, screen_h, weak));
        }
    }
    element.into_any_element()
}

/// One corner resize handle of the selected object. Dragging resizes with
/// the aspect locked; Shift at gesture start unlocks it (plan.md §13).
///
/// Positioned inside the (positioned) object div: offsets are object-local
/// screen px relative to the object's top-left corner.
fn handle_element(
    theme: &Theme,
    corner: Corner,
    object: ObjectId,
    width: f32,
    height: f32,
    weak: &WeakEntity<MarkApp>,
) -> gpui_kit::AnyElement {
    let half = HANDLE_SIZE / 2.;
    let (hx, hy) = match corner {
        Corner::TopLeft => (-half, -half),
        Corner::TopRight => (width - half, -half),
        Corner::BottomRight => (width - half, height - half),
        Corner::BottomLeft => (-half, height - half),
    };
    let press_weak = weak.clone();
    let drag_weak = weak.clone();
    div()
        .id(format!("mark-handle-{}-{object}", corner.label()))
        .test_support()
        .absolute()
        .left(px(hx - half))
        .top(px(hy - half))
        .size(px(HANDLE_SIZE))
        .rounded_sm()
        .bg(theme.background)
        .border_1()
        .border_color(theme.accent)
        // Keep the press inside the handle: bubbling to the canvas would
        // clear the selection and unmount the handle mid-press.
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            press_weak
                .update(cx, |app, _| {
                    app.handle_press(pointer_vec(event.position));
                })
                .ok();
        })
        .on_drag(
            ResizeObjectDrag,
            move |_: &ResizeObjectDrag, position, window, cx| {
                cx.stop_propagation();
                let aspect_lock = !window.modifiers().shift;
                drag_weak
                    .update(cx, |app, _| {
                        app.begin_resize(object, corner, aspect_lock, pointer_vec(position));
                    })
                    .ok();
                cx.new(|_| ResizeObjectDrag)
            },
        )
        .into_any_element()
}

/// Pointer position of a drag event as a Vec2 (screen px).
fn event_pointer<T: 'static>(event: &DragMoveEvent<T>) -> Vec2 {
    pointer_vec(event.event.position)
}

fn pointer_vec(position: Point<Pixels>) -> Vec2 {
    Vec2::new(f32::from(position.x), f32::from(position.y))
}

/// Wheel deltas normalized to screen-space pixels.
fn scroll_delta(event: &ScrollWheelEvent) -> (f32, f32) {
    const LINE_PX: f32 = 40.;
    match event.delta {
        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
        ScrollDelta::Lines(delta) => (delta.x * LINE_PX, delta.y * LINE_PX),
    }
}
