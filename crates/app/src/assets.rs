//! Signature/stamp assets panel (plan.md §10 right column, §12 library).
//!
//! Sections per asset kind, an add button per section importing through the
//! native picker, and asset rows that place the asset centered on the
//! current page with one click. Panel data arrives as an owned snapshot so
//! rendering never borrows the app entity mutably and immutably at once.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{
    ClickEvent, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    RenderImage, SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, img, px, rems,
};
use gpui_omarchy::{ButtonVariant, IconName, Theme, icon, icon_button};
use mark_core::{Asset, AssetId, AssetKind};

use crate::MarkApp;

/// Panel column width (logical px): preview + name + padding.
const PANEL_WIDTH: f32 = 240.;
/// Asset preview row height.
const PREVIEW_HEIGHT: f32 = 36.;

/// Owned per-frame snapshot of library state for the panel and canvas.
pub(crate) struct LibrarySnapshot {
    pub assets: Vec<Asset>,
    pub images: HashMap<AssetId, Arc<RenderImage>>,
    pub notice: Option<SharedString>,
}

/// The assets panel shown beside the canvas while a document is open.
pub(crate) fn panel(
    theme: &Theme,
    library: &LibrarySnapshot,
    window: &Window,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    let sections = [
        (AssetKind::Signature, IconName::Signature, "signature"),
        (AssetKind::Stamp, IconName::Stamp, "stamp"),
    ];

    div()
        .id("mark-assets")
        .flex()
        .flex_col()
        .min_h_0()
        .w(px(PANEL_WIDTH))
        .flex_shrink_0()
        .bg(theme.background)
        .border_l_1()
        .border_color(theme.border)
        .overflow_y_scroll()
        .child(
            div()
                .id("mark-assets-header")
                .flex()
                .items_center()
                .px(rems(0.75))
                .h(rems(2.))
                .flex_shrink_0()
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child("Assets"),
                ),
        )
        .children(library.notice.as_ref().map(|notice| {
            div()
                .id("mark-assets-notice")
                .px(rems(0.75))
                .pb(rems(0.5))
                .text_size(rems(0.6875))
                .text_color(theme.danger)
                .child(notice.clone())
        }))
        .children(sections.iter().map(|&(kind, kind_icon, label)| {
            section(theme, kind, kind_icon, label, library, window, cx)
        }))
}

/// One kind section: header with add button, then its asset rows.
fn section(
    theme: &Theme,
    kind: AssetKind,
    kind_icon: IconName,
    label: &str,
    library: &LibrarySnapshot,
    window: &Window,
    cx: &mut Context<MarkApp>,
) -> gpui_kit::AnyElement {
    let assets: Vec<&Asset> = library
        .assets
        .iter()
        .filter(|asset| asset.kind() == kind)
        .collect();

    let mut section = div()
        .id(format!("mark-assets-section-{label}"))
        .flex()
        .flex_col()
        .pb(rems(0.75))
        .child(
            div()
                .id(format!("mark-assets-section-header-{label}"))
                .flex()
                .items_center()
                .gap(rems(0.375))
                .px(rems(0.75))
                .py(rems(0.375))
                .child(
                    icon(kind_icon)
                        .size(rems(0.875))
                        .text_color(theme.secondary),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(kind.label()),
                )
                .child(div().flex_1())
                .child(
                    icon_button(
                        format!("mark-assets-add-{label}"),
                        IconName::Plus,
                        format!("Add {}", label),
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |app, _: &ClickEvent, _, cx| {
                            app.import_asset(kind, cx);
                        },
                    )),
                ),
        );

    if assets.is_empty() {
        section = section.child(
            div()
                .id(format!("mark-assets-empty-{label}"))
                .px(rems(0.75))
                .py(rems(0.375))
                .text_size(rems(0.6875))
                .text_color(theme.secondary)
                .child(format!("No {label}s yet. Add one to place it.")),
        );
    } else {
        section = section.children(
            assets
                .into_iter()
                .map(|asset| row(theme, asset, library.images.get(&asset.id()), window, cx)),
        );
    }
    section.into_any_element()
}

/// One asset row: aspect-correct preview plus name; click places the asset.
fn row(
    theme: &Theme,
    asset: &Asset,
    image: Option<&Arc<RenderImage>>,
    _window: &Window,
    cx: &mut Context<MarkApp>,
) -> gpui_kit::AnyElement {
    let id = asset.id();
    let preview_width = (PREVIEW_HEIGHT * asset.aspect()).clamp(16., 96.);
    let preview = match image {
        Some(image) => img(image.clone())
            .w(px(preview_width))
            .h(px(PREVIEW_HEIGHT))
            .into_any_element(),
        None => div()
            .w(px(preview_width))
            .h(px(PREVIEW_HEIGHT))
            .bg(theme.inset)
            .into_any_element(),
    };

    div()
        .id(format!("mark-asset-{id}"))
        .test_support()
        .flex()
        .items_center()
        .gap(rems(0.5))
        .px(rems(0.75))
        .py(rems(0.25))
        .rounded_md()
        .hover(|style| style.bg(theme.inset))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |app, _: &gpui_kit::MouseDownEvent, _, cx| app.place_asset(id, cx)),
        )
        .child(
            div()
                .flex_shrink_0()
                .border_1()
                .border_color(theme.border)
                .rounded_sm()
                .overflow_hidden()
                .child(preview),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme.foreground)
                .child(asset.name().to_owned()),
        )
        .into_any_element()
}
