//! Page-thumbnail sidebar (plan.md §10 layout, §11.2 lazy generation).
//!
//! A virtualized list: rows exist only for the visible range, thumbnails
//! are queued as ranges become visible, and the current page is marked by
//! its own row surface (accent border + brighter label).

use std::sync::Arc;

use gpui_kit::base::{ScrollbarAxis, v_virtual_list};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    RenderImage, Styled as _, Window, div, img, px, rems,
};
use gpui_omarchy::{Theme, scrollbar};

use crate::MarkApp;
use crate::viewer::{THUMBNAIL_DISPLAY_WIDTH, ViewerState};

/// Sidebar column width: thumbnail width plus symmetric row padding.
const SIDEBAR_WIDTH: f32 = THUMBNAIL_DISPLAY_WIDTH + 32.;

/// The thumbnails panel for the currently open document.
pub(crate) fn sidebar(
    theme: &Theme,
    viewer: &ViewerState,
    window: &Window,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    let item_sizes = viewer.thumbnail_item_sizes();
    let scroll_handle = viewer.scroll_handle.clone();
    let theme = theme.clone();

    div()
        .id("mark-thumbnails")
        .flex()
        .flex_col()
        .min_h_0()
        .w(px(SIDEBAR_WIDTH))
        .flex_shrink_0()
        .bg(theme.background)
        .border_r_1()
        .border_color(theme.border)
        .relative()
        .child(
            v_virtual_list(
                cx.entity(),
                "page-thumbnails",
                item_sizes,
                move |app: &mut MarkApp,
                      range: std::ops::Range<usize>,
                      _window: &mut Window,
                      cx: &mut Context<MarkApp>|
                      -> Vec<AnyElement> {
                    // Recording visibility is bookkeeping only; the
                    // resulting requests are deferred out of the frame.
                    if app.viewer_mut().note_visible_thumbnails(range.clone()) {
                        let entity = cx.entity();
                        cx.defer(move |cx| {
                            entity.update(cx, |app, cx| app.refresh_view(cx));
                        });
                    }
                    let current = app.viewer_ref().current_page();
                    let viewer = app.viewer_ref();
                    range
                        .map(|page| {
                            thumbnail_row(
                                cx,
                                &theme,
                                viewer.thumbnail(page as u32),
                                page as u32,
                                page as u32 == current,
                                viewer.page_size(page as u32),
                            )
                        })
                        .collect()
                },
            )
            .track_scroll(&scroll_handle)
            .size_full(),
        )
        .child(scrollbar(
            "page-thumbnails-scroll",
            ScrollbarAxis::Vertical,
            &scroll_handle,
            window,
            cx,
        ))
}

/// One sidebar row: the thumbnail (or its aspect-correct placeholder) and
/// the page number.
fn thumbnail_row(
    cx: &mut Context<MarkApp>,
    theme: &Theme,
    thumbnail: Option<&Arc<RenderImage>>,
    page: u32,
    is_current: bool,
    page_size: Option<mark_core::Vec2>,
) -> AnyElement {
    let height = page_size
        .map(|size| THUMBNAIL_DISPLAY_WIDTH * size.y / size.x.max(f32::EPSILON))
        .unwrap_or(THUMBNAIL_DISPLAY_WIDTH);
    let border = if is_current {
        theme.accent
    } else {
        theme.border
    };
    let thumbnail_element = match thumbnail {
        Some(image) => img(image.clone())
            .w(px(THUMBNAIL_DISPLAY_WIDTH))
            .h(px(height))
            .into_any_element(),
        None => div()
            .w(px(THUMBNAIL_DISPLAY_WIDTH))
            .h(px(height))
            .bg(theme.inset)
            .into_any_element(),
    };
    let label_color = if is_current {
        theme.bright
    } else {
        theme.secondary
    };

    div()
        .id(("page-thumbnail", page as usize))
        .flex()
        .flex_col()
        .items_center()
        .py(px(6.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |app, _, _, cx| app.go_to_page(page, cx)),
        )
        .child(
            div()
                .border_1()
                .border_color(border)
                .child(thumbnail_element),
        )
        .child(
            div()
                .mt(px(4.))
                .text_size(rems(0.6875))
                .text_color(label_color)
                .child((page + 1).to_string()),
        )
        .into_any_element()
}
