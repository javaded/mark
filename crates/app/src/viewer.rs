//! Page-viewer state: current page, zoom/pan, the page render cache, and
//! the thumbnail generation queue (plan.md §8, §11).
//!
//! This module is pure state plus computation — it never touches the PDFium
//! worker or GPUI contexts. `MarkApp` asks `ViewerState` what it *wants*
//! (which page at which width, which thumbnail next), spawns the requests,
//! and records the replies here. All screen/document math flows through
//! [`ViewTransform`] (plan.md §8).

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::{Pixels, RenderImage, ScrollStrategy, Size, px, size};
use mark_core::{Vec2, ViewTransform};

/// Discrete zoom ladder (plan.md §7: zoom range 50%–400%).
pub(crate) const ZOOM_STEPS: &[f32] = &[0.5, 0.67, 0.8, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0];

/// Render widths are bucketed so small zoom changes reuse the existing
/// bitmap (plan.md §11.1: reuse for small changes, re-render when the
/// resolution difference becomes significant).
const RENDER_BUCKET: u32 = 256;
const RENDER_WIDTH_MIN: u32 = 256;
const RENDER_WIDTH_MAX: u32 = 4096;

/// Cached page renders kept per document (plan.md §11.1 "bounded memory
/// cache"; entries are viewport-sized RGBA bitmaps).
const RENDER_CACHE_CAPACITY: usize = 12;

/// Thumbnail bitmap width (plan.md §11.2: ~160–240 px, low resolution).
pub(crate) const THUMBNAIL_RENDER_WIDTH: u32 = 240;
/// Thumbnail display width in the sidebar, logical pixels.
pub(crate) const THUMBNAIL_DISPLAY_WIDTH: f32 = 160.;

/// A cached render is reused while it covers at least this fraction of the
/// wanted width — one bucket below the exact request still looks sharp.
const RENDER_REUSE_FRACTION: f32 = 0.9;

/// Sidebar row geometry shared with the thumbnail list (plan.md §11.2).
const THUMB_ROW_PADDING: f32 = 6.;
const THUMB_ROW_GAP: f32 = 4.;
const THUMB_LABEL_HEIGHT: f32 = 18.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ZoomMode {
    /// Fit the whole page inside the viewport (default).
    Fit,
    /// An explicit zoom factor, 1.0 = 100% (1 pt → 1 logical px).
    Custom(f32),
}

/// An in-progress pointer pan of the canvas stage.
#[derive(Debug, Clone, Copy)]
struct PanDrag {
    start: Vec2,
    pan_start: Vec2,
}

pub(crate) struct ViewerState {
    page_sizes: Vec<Vec2>,
    /// Whether pages re-render on demand (PDFs); image documents display
    /// their source pixels at any zoom.
    renderable: bool,
    current_page: u32,
    zoom_mode: ZoomMode,
    pan: Vec2,
    /// Canvas-stage viewport: (width, height) in logical px plus scale
    /// factor. Unknown until the first frame measures the stage.
    viewport: Option<(f32, f32, f32)>,
    /// Page render cache keyed by (page, render width); `render_order`
    /// tracks recency for the bounded-eviction policy (plan.md §11.1).
    renders: HashMap<(u32, u32), Arc<RenderImage>>,
    render_order: Vec<(u32, u32)>,
    /// The page render currently requested from the worker, if any.
    pending_render: Option<(u32, u32)>,
    /// Thumbnails that have arrived; indexed by page.
    thumbnails: Vec<Option<Arc<RenderImage>>>,
    /// Thumbnail generation queue, driven by sidebar visibility; front is
    /// next (plan.md §11.2: lazy, progressive, current page first).
    thumb_queue: VecDeque<u32>,
    thumb_requested: HashSet<u32>,
    thumb_in_flight: Option<u32>,
    /// Sidebar page range seen in the last frame.
    visible_thumbs: Option<Range<usize>>,
    /// Sidebar scroll handle (scroll-to-current-page on navigation).
    pub(crate) scroll_handle: VirtualListScrollHandle,
    /// Precomputed sidebar row sizes (page aspect determines row height).
    thumbnail_item_sizes: Rc<Vec<Size<Pixels>>>,
    pan_drag: Option<PanDrag>,
}

impl ViewerState {
    /// Viewer for a PDF document: pages re-render at viewport resolution.
    pub(crate) fn new_pdf(page_sizes: Vec<Vec2>) -> Self {
        Self::build(page_sizes, true)
    }

    /// Viewer for an image document: a single page that never re-renders.
    pub(crate) fn new_image(page_size: Vec2) -> Self {
        Self::build(vec![page_size], false)
    }

    fn build(page_sizes: Vec<Vec2>, renderable: bool) -> Self {
        let thumbnail_item_sizes = Rc::new(
            page_sizes
                .iter()
                .map(|page| {
                    let thumb_height =
                        page.y * (THUMBNAIL_DISPLAY_WIDTH / page.x.max(f32::EPSILON));
                    let row =
                        thumb_height + THUMB_ROW_PADDING * 2. + THUMB_ROW_GAP + THUMB_LABEL_HEIGHT;
                    size(px(THUMBNAIL_DISPLAY_WIDTH), px(row))
                })
                .collect(),
        );
        let thumbnails = page_sizes.iter().map(|_| None).collect();
        Self {
            page_sizes,
            renderable,
            current_page: 0,
            zoom_mode: ZoomMode::Fit,
            pan: Vec2::new(0., 0.),
            viewport: None,
            renders: HashMap::new(),
            render_order: Vec::new(),
            pending_render: None,
            thumbnails,
            thumb_queue: VecDeque::new(),
            thumb_requested: HashSet::new(),
            thumb_in_flight: None,
            visible_thumbs: None,
            scroll_handle: VirtualListScrollHandle::new(),
            thumbnail_item_sizes,
            pan_drag: None,
        }
    }

    pub(crate) fn page_count(&self) -> usize {
        self.page_sizes.len()
    }

    pub(crate) fn current_page(&self) -> u32 {
        self.current_page
    }

    pub(crate) fn page_size(&self, page: u32) -> Option<Vec2> {
        self.page_sizes.get(page as usize).copied()
    }

    pub(crate) fn thumbnail_item_sizes(&self) -> Rc<Vec<Size<Pixels>>> {
        self.thumbnail_item_sizes.clone()
    }

    pub(crate) fn thumbnail(&self, page: u32) -> Option<&Arc<RenderImage>> {
        self.thumbnails.get(page as usize).and_then(Option::as_ref)
    }

    // ----- viewport ------------------------------------------------------------

    /// Records the canvas-stage viewport; true when it changed enough to
    /// affect fit zoom or render sizing.
    pub(crate) fn set_viewport(&mut self, width: f32, height: f32, scale_factor: f32) -> bool {
        let changed = self.viewport.is_none_or(|(w, h, s)| {
            (w - width).abs() > 0.5 || (h - height).abs() > 0.5 || s != scale_factor
        });
        if changed {
            self.viewport = Some((width, height, scale_factor));
        }
        changed
    }

    /// Zoom factor that fits the current page inside the viewport.
    fn fit_zoom(&self) -> Option<f32> {
        let (viewport_w, viewport_h, _) = self.viewport?;
        let page = self.page_size(self.current_page)?;
        let zoom_x = viewport_w / page.x.max(f32::EPSILON);
        let zoom_y = viewport_h / page.y.max(f32::EPSILON);
        Some(zoom_x.min(zoom_y).max(0.01))
    }

    pub(crate) fn effective_zoom(&self) -> Option<f32> {
        match self.zoom_mode {
            ZoomMode::Fit => self.fit_zoom(),
            ZoomMode::Custom(zoom) => Some(zoom),
        }
    }

    /// Effective zoom rounded for display, e.g. 78 (for "78%").
    pub(crate) fn zoom_percent(&self) -> Option<u32> {
        self.effective_zoom()
            .map(|zoom| (zoom * 100.).round() as u32)
    }

    /// The clamped document→screen transform for the current view.
    pub(crate) fn transform(&self) -> Option<ViewTransform> {
        let zoom = self.effective_zoom()?;
        let page = self.page_size(self.current_page)?;
        let (pan_x, pan_y) = self.clamped_pan(page, zoom);
        Some(ViewTransform::new(zoom, pan_x, pan_y))
    }

    /// Pan clamping: content smaller than the viewport stays centered;
    /// overflowing content may never be pushed fully out of view.
    fn clamped_pan(&self, page: Vec2, zoom: f32) -> (f32, f32) {
        let Some((viewport_w, viewport_h, _)) = self.viewport else {
            return (self.pan.x, self.pan.y);
        };
        let clamp_axis = |pan: f32, content: f32, viewport: f32| {
            if content <= viewport {
                (viewport - content) / 2.
            } else {
                pan.clamp(viewport - content, 0.)
            }
        };
        (
            clamp_axis(self.pan.x, page.x * zoom, viewport_w),
            clamp_axis(self.pan.y, page.y * zoom, viewport_h),
        )
    }

    // ----- navigation ----------------------------------------------------------

    /// Navigates to `page` (clamped), resetting pan. True when the page
    /// changed.
    pub(crate) fn navigate_to(&mut self, page: u32) -> bool {
        let page = page.min(self.final_page_index());
        if page == self.current_page {
            return false;
        }
        self.current_page = page;
        self.pan = Vec2::new(0., 0.);
        self.scroll_handle
            .scroll_to_item(page as usize, ScrollStrategy::Center);
        true
    }

    pub(crate) fn next_page(&mut self) -> bool {
        self.navigate_to(self.current_page + 1)
    }

    pub(crate) fn previous_page(&mut self) -> bool {
        self.navigate_to(self.current_page.saturating_sub(1))
    }

    pub(crate) fn first_page(&mut self) -> bool {
        self.navigate_to(0)
    }

    pub(crate) fn last_page(&mut self) -> bool {
        self.navigate_to(u32::MAX)
    }

    fn final_page_index(&self) -> u32 {
        (self.page_sizes.len() as u32).saturating_sub(1)
    }

    pub(crate) fn has_next_page(&self) -> bool {
        self.current_page < self.final_page_index()
    }

    pub(crate) fn has_previous_page(&self) -> bool {
        self.current_page > 0
    }

    // ----- zoom ----------------------------------------------------------------

    /// Steps zoom to the next ladder rung above the effective zoom.
    pub(crate) fn zoom_in(&mut self) {
        self.zoom_to_step(true);
    }

    /// Steps zoom to the next ladder rung below the effective zoom.
    pub(crate) fn zoom_out(&mut self) {
        self.zoom_to_step(false);
    }

    /// Returns to fit-to-viewport zoom with pan centered.
    pub(crate) fn zoom_fit(&mut self) {
        self.zoom_mode = ZoomMode::Fit;
        self.pan = Vec2::new(0., 0.);
    }

    fn zoom_to_step(&mut self, up: bool) {
        if let Some(current) = self.effective_zoom() {
            self.zoom_around_viewport_center(step_zoom(current, up));
        }
    }

    /// Changes zoom while keeping the document point under the viewport
    /// center fixed (the standard document-viewer zoom anchor).
    fn zoom_around_viewport_center(&mut self, zoom: f32) {
        if let Some((viewport_w, viewport_h, _)) = self.viewport
            && let Some(page) = self.page_size(self.current_page)
        {
            let old_zoom = self.effective_zoom().unwrap_or(zoom);
            let (pan_x, pan_y) = self.clamped_pan(page, old_zoom);
            let center_doc_x = (viewport_w / 2. - pan_x) / old_zoom;
            let center_doc_y = (viewport_h / 2. - pan_y) / old_zoom;
            self.pan = Vec2::new(
                viewport_w / 2. - center_doc_x * zoom,
                viewport_h / 2. - center_doc_y * zoom,
            );
        }
        self.zoom_mode = ZoomMode::Custom(zoom);
    }

    // ----- pan -----------------------------------------------------------------

    /// Pans by screen-space pixels (wheel or drag delta).
    pub(crate) fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan = Vec2::new(self.pan.x + dx, self.pan.y + dy);
    }

    pub(crate) fn pan_drag_started(&mut self, position: Vec2) {
        self.pan_drag = Some(PanDrag {
            start: position,
            pan_start: self.pan,
        });
    }

    pub(crate) fn pan_drag_moved(&mut self, position: Vec2) {
        if let Some(drag) = self.pan_drag {
            self.pan = Vec2::new(
                drag.pan_start.x + position.x - drag.start.x,
                drag.pan_start.y + position.y - drag.start.y,
            );
        }
    }

    pub(crate) fn pan_drag_ended(&mut self) {
        self.pan_drag = None;
    }

    // ----- page render cache (plan.md §11.1) --------------------------------------

    /// The best cached render to display for `page`: highest resolution
    /// wins, and reading refreshes recency.
    pub(crate) fn image_for(&mut self, page: u32) -> Option<Arc<RenderImage>> {
        let best = self
            .render_order
            .iter()
            .filter(|(cached_page, _)| *cached_page == page)
            .max_by_key(|(_, width)| *width)
            .copied()?;
        self.touch_render(&best);
        self.renders.get(&best).cloned()
    }

    /// Seeds a page image that needs no worker (image documents); the same
    /// image doubles as the page's thumbnail.
    pub(crate) fn seed_page_image(&mut self, page: u32, width: u32, image: Arc<RenderImage>) {
        self.insert_render(page, width, image.clone());
        if let Some(slot) = self.thumbnails.get_mut(page as usize) {
            *slot = Some(image);
        }
    }

    /// The render the worker should produce next: `Some((page, width))`
    /// when the cache cannot satisfy the current view at acceptable
    /// resolution and no matching request is already in flight.
    pub(crate) fn wanted_render(&self) -> Option<(u32, u32)> {
        if !self.renderable {
            return None;
        }
        let (_, _, scale_factor) = self.viewport?;
        let zoom = self.effective_zoom()?;
        let page = self.page_size(self.current_page)?;
        let wanted = bucket_render_width(page.x * zoom * scale_factor);
        let satisfiable = self.render_order.iter().any(|(page, width)| {
            *page == self.current_page && *width as f32 >= wanted as f32 * RENDER_REUSE_FRACTION
        });
        if satisfiable || self.pending_render == Some((self.current_page, wanted)) {
            return None;
        }
        Some((self.current_page, wanted))
    }

    pub(crate) fn mark_render_requested(&mut self, page: u32, width: u32) {
        self.pending_render = Some((page, width));
    }

    /// A render reply arrived for a request we no longer expect: forget the
    /// pending slot so `wanted_render` can issue a fresh one.
    pub(crate) fn clear_pending_render(&mut self, page: u32, width: u32) {
        if self.pending_render == Some((page, width)) {
            self.pending_render = None;
        }
    }

    pub(crate) fn record_render(&mut self, page: u32, width: u32, image: Arc<RenderImage>) {
        self.clear_pending_render(page, width);
        self.insert_render(page, width, image);
    }

    pub(crate) fn page_render_pending(&self) -> bool {
        self.pending_render.is_some()
    }

    fn insert_render(&mut self, page: u32, width: u32, image: Arc<RenderImage>) {
        let key = (page, width);
        self.touch_render(&key);
        self.renders.insert(key, image);
        while self.render_order.len() > RENDER_CACHE_CAPACITY {
            let evicted = self.render_order.remove(0);
            self.renders.remove(&evicted);
        }
    }

    fn touch_render(&mut self, key: &(u32, u32)) {
        if let Some(index) = self.render_order.iter().position(|k| k == key) {
            self.render_order.remove(index);
        }
        self.render_order.push(*key);
    }

    // ----- thumbnails (plan.md §11.2) ----------------------------------------------

    /// Records which thumbnails the sidebar is showing; pages inside
    /// `range` without a thumbnail are queued for generation. Returns true
    /// when the recorded range changed.
    pub(crate) fn note_visible_thumbnails(&mut self, range: Range<usize>) -> bool {
        let changed = self.visible_thumbs.as_ref() != Some(&range);
        if changed {
            self.visible_thumbs = Some(range.clone());
        }
        for page in range {
            if self.thumbnail_missing(page as u32) {
                self.enqueue_thumbnail(page as u32);
            }
        }
        changed
    }

    fn thumbnail_missing(&self, page: u32) -> bool {
        self.thumbnails
            .get(page as usize)
            .is_some_and(Option::is_none)
            && !self.thumb_requested.contains(&page)
    }

    fn enqueue_thumbnail(&mut self, page: u32) {
        if !self.thumb_queue.contains(&page) {
            self.thumb_queue.push_back(page);
        }
    }

    /// Pushes `page` to the front of the thumbnail queue — the current
    /// page is the most useful thumbnail to have (plan.md §11.2).
    pub(crate) fn prioritize_thumbnail(&mut self, page: u32) {
        if self.thumbnail_missing(page) && !self.thumb_queue.contains(&page) {
            self.thumb_queue.push_front(page);
        }
    }

    /// Pops the next thumbnail to generate. Thumbnails never compete with
    /// the current page's render (plan.md §11.2 priority rule) and only one
    /// is in flight at a time, so generation stays progressive.
    pub(crate) fn next_thumbnail_request(&mut self) -> Option<u32> {
        if self.page_render_pending() || self.thumb_in_flight.is_some() {
            return None;
        }
        while let Some(page) = self.thumb_queue.pop_front() {
            if self.thumbnail_missing(page) {
                self.thumb_requested.insert(page);
                self.thumb_in_flight = Some(page);
                return Some(page);
            }
        }
        None
    }

    pub(crate) fn record_thumbnail(&mut self, page: u32, image: Arc<RenderImage>) {
        if let Some(slot) = self.thumbnails.get_mut(page as usize) {
            *slot = Some(image);
        }
        if self.thumb_in_flight == Some(page) {
            self.thumb_in_flight = None;
        }
        self.thumb_requested.remove(&page);
    }

    /// A thumbnail render failed: free the in-flight slot without
    /// clearing `requested`, so the failed page is not retried forever.
    pub(crate) fn clear_thumbnail_in_flight(&mut self, page: u32) {
        if self.thumb_in_flight == Some(page) {
            self.thumb_in_flight = None;
        }
    }
}

/// Rounds a wanted pixel width up to a render bucket.
pub(crate) fn bucket_render_width(wanted: f32) -> u32 {
    ((wanted.ceil().max(1.) as u32).div_ceil(RENDER_BUCKET) * RENDER_BUCKET)
        .clamp(RENDER_WIDTH_MIN, RENDER_WIDTH_MAX)
}

/// The next ladder rung beyond `current` in the given direction (plan.md §7
/// zoom range).
pub(crate) fn step_zoom(current: f32, up: bool) -> f32 {
    let pick = if up {
        ZOOM_STEPS.iter().find(|step| **step > current + 1e-3)
    } else {
        ZOOM_STEPS.iter().rev().find(|step| **step < current - 1e-3)
    };
    pick.copied().unwrap_or(if up {
        ZOOM_STEPS[ZOOM_STEPS.len() - 1]
    } else {
        ZOOM_STEPS[0]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter_pages(count: usize) -> Vec<Vec2> {
        vec![Vec2::new(612., 792.); count]
    }

    fn viewer_with_viewport(pages: Vec<Vec2>, w: f32, h: f32) -> ViewerState {
        let mut viewer = ViewerState::new_pdf(pages);
        assert!(viewer.set_viewport(w, h, 1.));
        viewer
    }

    fn image(width: u32) -> Arc<RenderImage> {
        Arc::new(RenderImage::new([image::Frame::new(
            image::RgbaImage::new(width, 4),
        )]))
    }

    fn image_width(image: &Arc<RenderImage>) -> u32 {
        image.size(0).width.0 as u32
    }

    #[test]
    fn zoom_ladder_steps_and_clamps() {
        assert_eq!(step_zoom(1.0, true), 1.25);
        assert_eq!(step_zoom(1.0, false), 0.8);
        assert_eq!(step_zoom(0.4, true), 0.5);
        assert_eq!(step_zoom(0.5, false), 0.5);
        assert_eq!(step_zoom(4.0, true), 4.0);
        assert_eq!(step_zoom(5.0, true), 4.0);
        assert_eq!(step_zoom(4.0, false), 3.0);
    }

    #[test]
    fn buckets_round_up_and_clamp() {
        assert_eq!(bucket_render_width(1.), 256);
        assert_eq!(bucket_render_width(612.), 768);
        assert_eq!(bucket_render_width(1024.), 1024);
        assert_eq!(bucket_render_width(1025.), 1280);
        assert_eq!(bucket_render_width(999_999.), 4096);
    }

    #[test]
    fn fit_zoom_fits_the_smaller_axis() {
        let viewer = viewer_with_viewport(letter_pages(1), 800., 600.);
        assert!((viewer.effective_zoom().unwrap() - 600. / 792.).abs() < 1e-4);
        let viewer = viewer_with_viewport(letter_pages(1), 500., 900.);
        assert!((viewer.effective_zoom().unwrap() - 500. / 612.).abs() < 1e-4);
    }

    #[test]
    fn pan_is_centered_when_content_fits_and_clamped_when_it_overflows() {
        let mut viewer = viewer_with_viewport(letter_pages(1), 800., 600.);
        let page = viewer.page_size(0).unwrap();
        let zoom = viewer.effective_zoom().unwrap();

        // Fit: centered regardless of pan drift.
        viewer.pan_by(500., -300.);
        let transform = viewer.transform().unwrap();
        assert_eq!(transform.pan_x, (800. - page.x * zoom) / 2.);
        assert_eq!(transform.pan_y, (600. - page.y * zoom) / 2.);

        // 200% overflows both axes: pan may not push the page out of view.
        viewer.zoom_around_viewport_center(2.0);
        viewer.pan_by(-10_000., 10_000.);
        let transform = viewer.transform().unwrap();
        assert_eq!(transform.pan_x, 800. - page.x * 2.);
        assert_eq!(transform.pan_y, 0.);
    }

    #[test]
    fn zoom_preserves_the_document_point_under_the_viewport_center() {
        let mut viewer = viewer_with_viewport(letter_pages(1), 800., 600.);
        viewer.zoom_around_viewport_center(1.0);
        let before = viewer.transform().unwrap();
        let center_doc = (
            (400. - before.pan_x) / before.zoom,
            (300. - before.pan_y) / before.zoom,
        );
        viewer.zoom_around_viewport_center(4.0);
        let after = viewer.transform().unwrap();
        let mapped = (
            (400. - after.pan_x) / after.zoom,
            (300. - after.pan_y) / after.zoom,
        );
        assert!((center_doc.0 - mapped.0).abs() < 1e-2);
        assert!((center_doc.1 - mapped.1).abs() < 1e-2);
    }

    #[test]
    fn navigation_clamps_resets_pan_and_marks_bounds() {
        let mut viewer = viewer_with_viewport(letter_pages(3), 800., 600.);
        viewer.zoom_around_viewport_center(2.0);
        viewer.pan_by(120., 80.);
        assert!(viewer.next_page());
        assert_eq!(viewer.current_page(), 1);
        assert_eq!(viewer.pan, Vec2::new(0., 0.));
        viewer.navigate_to(99);
        assert_eq!(viewer.current_page(), 2);
        assert!(!viewer.has_next_page());
        assert!(!viewer.next_page());
        assert!(viewer.first_page());
        assert!(!viewer.has_previous_page());
    }

    #[test]
    fn wanted_render_bucketizes_and_deduplicates() {
        let mut viewer = viewer_with_viewport(letter_pages(1), 800., 600.);
        // 612 pt × fit(0.7575) × scale 1 ≈ 464 px → bucket 512.
        assert_eq!(viewer.wanted_render(), Some((0, 512)));

        viewer.mark_render_requested(0, 512);
        assert_eq!(viewer.wanted_render(), None);

        // A reply at the exact width satisfies the want.
        viewer.record_render(0, 512, image(512));
        assert_eq!(viewer.wanted_render(), None);

        // Zooming in past the reuse band wants the next bucket
        // (612 pt × 2.0 = 1224 px → 1280).
        viewer.zoom_around_viewport_center(2.0);
        assert_eq!(viewer.wanted_render(), Some((0, 1280)));

        // The still-cached 512-wide render is reused for display.
        assert_eq!(viewer.image_for(0).map(|img| image_width(&img)), Some(512));
    }

    #[test]
    fn image_documents_never_request_renders() {
        let mut viewer = ViewerState::new_image(Vec2::new(640., 400.));
        viewer.set_viewport(800., 600., 1.);
        assert_eq!(viewer.wanted_render(), None);
        let seeded = image(640);
        viewer.seed_page_image(0, 640, seeded.clone());
        assert!(Arc::ptr_eq(viewer.thumbnail(0).unwrap(), &seeded));
        assert_eq!(viewer.image_for(0).map(|img| image_width(&img)), Some(640));
    }

    #[test]
    fn render_cache_evicts_least_recent_beyond_capacity() {
        let mut viewer = ViewerState::new_pdf(letter_pages(20));
        for page in 0..(RENDER_CACHE_CAPACITY as u32 + 2) {
            viewer.insert_render(page, 512, image(512));
        }
        // 14 inserted, capacity 12: pages 0 and 1 (least recent) are gone.
        assert_eq!(viewer.render_order.len(), RENDER_CACHE_CAPACITY);
        assert!(viewer.image_for(0).is_none());
        assert!(viewer.image_for(1).is_none());
        assert!(viewer.image_for(2).is_some());
    }

    #[test]
    fn thumbnail_queue_is_gated_lazy_and_current_page_first() {
        let mut viewer = ViewerState::new_pdf(letter_pages(6));
        assert!(viewer.note_visible_thumbnails(0..2));
        assert!(!viewer.note_visible_thumbnails(0..2)); // idempotent recording

        // A pending page render gates thumbnail generation.
        viewer.mark_render_requested(3, 512);
        assert_eq!(viewer.next_thumbnail_request(), None);
        viewer.record_render(3, 512, image(512));

        assert_eq!(viewer.next_thumbnail_request(), Some(0));
        assert_eq!(viewer.next_thumbnail_request(), None); // one in flight
        viewer.record_thumbnail(0, image(240));
        assert_eq!(viewer.next_thumbnail_request(), Some(1));
        viewer.record_thumbnail(1, image(240));
        assert_eq!(viewer.next_thumbnail_request(), None); // queue drained

        // The current page jumps the queue.
        viewer.navigate_to(4);
        viewer.prioritize_thumbnail(4);
        viewer.note_visible_thumbnails(3..6);
        assert_eq!(viewer.next_thumbnail_request(), Some(4));
        viewer.record_thumbnail(4, image(240));
        assert_eq!(viewer.next_thumbnail_request(), Some(3));
        viewer.record_thumbnail(3, image(240));
        assert_eq!(viewer.next_thumbnail_request(), Some(5));
        viewer.record_thumbnail(5, image(240));
        assert_eq!(viewer.next_thumbnail_request(), None);
    }

    #[test]
    fn thumbnail_row_sizes_follow_page_aspect() {
        let viewer = ViewerState::new_pdf(vec![
            Vec2::new(612., 792.),
            Vec2::new(792., 612.), // landscape renders shorter
        ]);
        let sizes = viewer.thumbnail_item_sizes();
        let expected = THUMBNAIL_DISPLAY_WIDTH * 792. / 612.
            + THUMB_ROW_PADDING * 2.
            + THUMB_ROW_GAP
            + THUMB_LABEL_HEIGHT;
        assert_eq!(sizes[0].width, px(THUMBNAIL_DISPLAY_WIDTH));
        assert!(
            (f32::from(sizes[0].height) - expected).abs() < 0.01,
            "portrait row height {} != expected {expected}",
            sizes[0].height
        );
        assert!(sizes[1].height < sizes[0].height);
    }
}
