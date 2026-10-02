//! Document canvas: renders the open page fitted to the viewport
//! (plan.md §10, §11).

use std::sync::Arc;

use gpui_kit::{
    InteractiveElement as _, IntoElement, ParentElement as _, RenderImage, Styled as _, div, img,
    rems,
};
use gpui_omarchy::Theme;

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

/// The document area: the current page centered and fitted.
///
/// `img` derives its aspect ratio from the image and both dimensions stay
/// auto, so `max_w_full`/`max_h_full` letterbox it inside the viewport
/// without distorting the page.
pub(crate) fn page(theme: &Theme, image: &Arc<RenderImage>) -> impl IntoElement {
    div()
        .id("mark-canvas")
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .p(rems(1.5))
        .child(
            img(image.clone())
                .max_w_full()
                .max_h_full()
                .border_1()
                .border_color(theme.border)
                .shadow_lg(),
        )
}
