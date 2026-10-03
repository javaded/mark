//! Document canvas: the current page drawn at the viewer's zoom and pan
//! inside a viewport-sized stage (plan.md §10, §11).
//!
//! A `canvas` element probe measures the stage each frame; the resulting
//! viewport drives fit zoom and render sizing (§11). Wheel and drag pan the
//! page; the clamped view transform keeps the page reachable.

use std::sync::Arc;

use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, Empty, InteractiveElement as _,
    IntoElement, MouseButton, MouseUpEvent, ParentElement as _, Pixels, Point, Render, RenderImage,
    ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, canvas, div, img, px, rems,
};
use gpui_omarchy::Theme;
use mark_core::{Vec2, ViewTransform};

use crate::MarkApp;
use crate::viewer::ViewerState;

/// Drag value marking an active canvas pan (GPUI drag protocol).
struct PanCanvas;

impl Render for PanCanvas {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A placed image object resolved for drawing: page-space placement plus
/// the cached asset bitmap (absent until its decode lands).
pub(crate) struct PlacedObject {
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
/// view transform, the objects placed on it, and the pan interactions.
pub(crate) fn stage(
    theme: &Theme,
    weak: &WeakEntity<MarkApp>,
    viewer: &mut ViewerState,
    placed: &[PlacedObject],
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
                ))
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
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.viewer_mut().pan_drag_ended();
                        cx.notify();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, _| {
                        this.viewer_mut().pan_drag_ended();
                    }),
                ),
        )
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
                        .map(|object| object_element(theme, &transform, object)),
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
/// page-space rect through the view transform; selected objects carry the
/// accent outline (plan.md §13; handles and dragging arrive in Phase 7).
fn object_element(
    theme: &Theme,
    transform: &ViewTransform,
    object: &PlacedObject,
) -> gpui_kit::AnyElement {
    let (x, y) = transform.document_to_screen(object.x, object.y);
    let mut element = div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(object.width * transform.zoom))
        .h(px(object.height * transform.zoom))
        .opacity(object.opacity);
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
    }
    element.into_any_element()
}

/// Wheel deltas normalized to screen-space pixels.
fn scroll_delta(event: &ScrollWheelEvent) -> (f32, f32) {
    const LINE_PX: f32 = 40.;
    match event.delta {
        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
        ScrollDelta::Lines(delta) => (delta.x * LINE_PX, delta.y * LINE_PX),
    }
}
