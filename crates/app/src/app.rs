//! Main application view: header bar, workspace states, open flow, page
//! navigation, zoom, and the render-request loop.
//!
//! Phases 3–4 opened images and PDFs; Phase 5 adds the viewer: lazy
//! thumbnails (§11.2), keyboard + status-bar navigation, and zoom/pan/fit
//! with viewport-sized re-rendering (§11.1). All PDFium work stays on the
//! worker thread (§6.4); this view only decides *what* to request and
//! records replies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui_kit::base::{
    AlertDialogAction, AlertDialogCancel, ToastManager, ToastMotion, ToastOptions,
};
use gpui_kit::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, Render, SharedString, Styled as _, Task,
    TestSupportExt as _, Window, div, rems,
};
use gpui_omarchy::{
    ActiveTheme, ButtonVariant, IconName, MenuItem, Theme, button, dialog_button,
    dialog_description, dialog_popup, dialog_title, focus_scope, icon, icon_button, menu,
    with_tooltip,
};
use mark_core::{
    AddObject, AssetId, AssetKind, DeleteObject, DocumentObject, DocumentSession, DuplicateObject,
    DuplicateToPage, ImageObject, ObjectId, ObjectKind, Rect, ResizeObject, Vec2,
};
use mark_export::export::{PngOverlay, compose_png_page, signed_destination};
use mark_export::library::AssetLibrary;
use mark_export::recent::RecentDocuments;
use mark_pdf::{ImageOverlay, PdfDocumentHandle, PdfExport, PdfWorker, RenderedPage};

use crate::assets;
use crate::canvas;
use crate::thumbnails;
use crate::viewer::ViewerState;
use crate::{
    ClearSelection as ClearSelectionAction, Copy as CopyAction,
    DeleteSelected as DeleteSelectedAction, Duplicate as DuplicateAction, Export as ExportAction,
    FirstPage as FirstPageAction, LastPage as LastPageAction, NextPage as NextPageAction,
    OpenDocument, Paste as PasteAction, PreviousPage as PreviousPageAction, Quit as QuitAction,
    Redo as RedoAction, Undo as UndoAction, ZoomFit as ZoomFitAction, ZoomIn as ZoomInAction,
    ZoomOut as ZoomOutAction,
};

/// How far a duplicate or the first paste lands from its source, in page
/// units (plan.md §13.2: offset near the selection, never exactly on top).
const DUPLICATE_OFFSET: f32 = 16.;
/// Each consecutive paste lands one more step out, so repeats cascade.
const PASTE_STEP: f32 = 16.;

/// One placed object awaiting its asset bytes, in display-space page
/// coordinates — the format-independent export input (plan.md §16).
struct OverlaySpec {
    rect: Rect,
    asset: AssetId,
    opacity: f32,
}

/// Feedback for the export flow (plan.md §16, §6.4): live progress while
/// the worker writes, then the outcome shown until the next export or
/// document switch (Phase 10 delivers the richer notification surfaces).
#[derive(Clone, Debug)]
pub(crate) enum ExportStatus {
    /// Pages written / pages total.
    Running {
        done: u32,
        total: u32,
    },
    Done {
        name: SharedString,
    },
    Failed(SharedString),
}

/// What runs once the user resolves the discard-changes dialog
/// (plan.md §16: closing a dirty document prompts Export… / Discard /
/// Cancel).
#[derive(Clone, Debug)]
pub(crate) enum PendingAction {
    /// Opens another document, dropping the dirty one.
    Open(PathBuf),
    /// Quits the application (window close and Ctrl+Q alike: single-window
    /// app — closing the window ends the process).
    Quit,
}

/// One transient toast notice (plan.md §10, §17): asynchronous status that
/// requires no decision.
#[derive(Clone, Debug)]
struct ToastNotice {
    message: SharedString,
    kind: ToastKind,
}

#[derive(Clone, Copy, Debug)]
enum ToastKind {
    Success,
    Info,
}

impl ToastKind {
    fn icon(self) -> IconName {
        match self {
            Self::Success => IconName::Check,
            Self::Info => IconName::Info,
        }
    }

    fn color(self, theme: &Theme) -> gpui_kit::Hsla {
        match self {
            Self::Success => theme.accent,
            Self::Info => theme.secondary,
        }
    }
}

/// How long a toast stays on screen before dismissing itself.
const TOAST_TIMEOUT: Duration = Duration::from_secs(4);
/// The toast lifecycle clock's tick.
const TOAST_TICK: Duration = Duration::from_millis(200);

/// Everything needed to display an opened document.
///
/// `viewer` owns view state (current page, zoom/pan, render cache,
/// thumbnails); document content lives in the session (plan.md §7.1);
/// `selected_object` is the editor selection on this document.
pub struct OpenedDocument {
    session: DocumentSession,
    /// Set for PDF documents: the worker-side handle for render requests
    /// and closing.
    pdf: Option<PdfDocumentHandle>,
    viewer: ViewerState,
    selected_object: Option<ObjectId>,
    /// Pointer position of the last press inside an object or handle:
    /// gestures anchor here, not at GPUI's drag-activation point (which
    /// fires after ~2px of movement and would swallow the first step).
    press_pointer: Option<Vec2>,
    /// In-progress move/resize; the preview rect overlays the stored
    /// geometry until pointer release commits one command (§14).
    pub(crate) gesture: Option<crate::manipulation::Gesture>,
}

impl OpenedDocument {
    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    /// The editor selection (test observation).
    #[cfg(test)]
    pub(crate) fn selected_object(&self) -> Option<ObjectId> {
        self.selected_object
    }

    /// View state (zoom/pan) access for gesture math in tests.
    #[cfg(test)]
    pub(crate) fn viewer(&self) -> &ViewerState {
        &self.viewer
    }
}

enum OpenState {
    Empty,
    Opening { name: SharedString },
    Failed { name: SharedString },
    Opened(Box<OpenedDocument>),
}

pub struct MarkApp {
    open: OpenState,
    pdf: Arc<PdfWorker>,
    /// Signature/stamp library, persisted in the app data directory
    /// (plan.md §12). App-level: it outlives any single document.
    library: AssetLibrary,
    /// Import/open failures surfaced in the assets panel (plan.md §17 copy).
    library_notice: Option<SharedString>,
    /// Decoded asset bitmaps for the panel and canvas, keyed by asset.
    asset_images: HashMap<AssetId, Arc<gpui_kit::RenderImage>>,
    /// The copied object (Ctrl+C), pastable onto any page of any open
    /// document (plan.md §13.2).
    clipboard: Option<ImageObject>,
    /// How many pastes happened since the copy: each lands one step further
    /// out so repeated Ctrl+V never stacks exactly on top (plan.md §13.2).
    paste_count: u32,
    /// Export feedback: progress while a job runs, then the outcome.
    export: Option<ExportStatus>,
    /// The action waiting on the discard-changes dialog (plan.md §16).
    confirm: Option<PendingAction>,
    /// The pending action to resume after an export triggered from the
    /// discard dialog completes.
    resume_after_export: Option<PendingAction>,
    /// Focus for the discard-changes dialog (focused while open, returns
    /// to the app scope on close).
    confirm_focus: FocusHandle,
    /// Recently opened documents (plan.md §18), persisted as JSON in the
    /// app config directory.
    recent: RecentDocuments,
    /// Where `recent` persists; `None` degrades to in-memory only.
    recent_path: Option<PathBuf>,
    /// Transient notices, newest last (plan.md §10).
    toasts: ToastManager<u64, ToastNotice>,
    /// Keeps `toasts` advancing while any is mounted; `None` when idle.
    toast_clock: Option<Task<()>>,
    next_toast_id: u64,
    /// The window title as last pushed to the platform: titles are synced
    /// opportunistically in `render` (every mutation notifies), but the
    /// platform call is made only on change.
    window_title: String,
    /// Keyboard focus for the whole app: actions (navigation, zoom, open)
    /// dispatch through the focused node.
    focus_handle: FocusHandle,
}

impl MarkApp {
    pub fn new(pdf: Arc<PdfWorker>, cx: &mut Context<Self>) -> Self {
        let root = platform::app_data_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("mark-library-fallback"));
        let recent_path = platform::app_config_dir().map(|dir| dir.join("recent.json"));
        Self::with_storage(pdf, root, recent_path, cx)
    }

    /// Constructor with the library root injected: tests use a throwaway
    /// directory (recents ride beside the library for per-test isolation).
    #[cfg(test)]
    pub(crate) fn with_library_root(
        pdf: Arc<PdfWorker>,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let recent_path = root.join("recent.json");
        Self::with_storage(pdf, root, Some(recent_path), cx)
    }

    /// Constructor with both persistence locations injected.
    pub(crate) fn with_storage(
        pdf: Arc<PdfWorker>,
        library_root: PathBuf,
        recent_path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (library, notice) = Self::open_library(&library_root);
        // Recents are derived convenience data, not user assets: a damaged
        // file degrades to an empty list without a notice (the next open
        // overwrites it) — unlike the library, which surfaces its damage.
        let recent = recent_path
            .as_deref()
            .and_then(|path| RecentDocuments::open(path).ok())
            .unwrap_or_default();
        let mut app = Self {
            open: OpenState::Empty,
            pdf,
            library,
            library_notice: notice,
            asset_images: HashMap::new(),
            clipboard: None,
            paste_count: 0,
            export: None,
            confirm: None,
            resume_after_export: None,
            confirm_focus: cx.focus_handle(),
            recent,
            recent_path,
            toasts: ToastManager::new(ToastMotion::sonner()),
            toast_clock: None,
            next_toast_id: 0,
            window_title: String::new(),
            focus_handle: cx.focus_handle(),
        };
        app.load_asset_images(cx);
        app
    }

    /// Opens the persisted library; a damaged manifest degrades to an
    /// empty session-only library behind a notice rather than hiding the
    /// app behind an error (plan.md §17).
    fn open_library(root: &Path) -> (AssetLibrary, Option<SharedString>) {
        match AssetLibrary::open(root) {
            Ok(library) => (library, None),
            Err(_) => (
                AssetLibrary::open(&std::env::temp_dir().join("mark-library-fallback"))
                    .expect("fresh temp library opens"),
                Some("The signature library could not be read.".into()),
            ),
        }
    }

    /// Decodes any asset bitmaps not yet cached (startup and post-import).
    fn load_asset_images(&mut self, cx: &mut Context<Self>) {
        let root = self.library.root().to_path_buf();
        let ids: Vec<AssetId> = self
            .library
            .assets()
            .iter()
            .map(|asset| asset.id())
            .filter(|id| !self.asset_images.contains_key(id))
            .collect();
        for id in ids {
            let root = root.clone();
            cx.spawn(async move |this, cx| {
                let image = cx
                    .background_executor()
                    .spawn(async move { mark_export::library::load_asset_image(&root, id).ok() })
                    .await;
                if let Some(rgba) = image {
                    this.update(cx, |app, cx| {
                        app.asset_images
                            .insert(id, Arc::new(canvas::render_image(&rgba)));
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    pub(crate) fn viewer_ref(&self) -> &ViewerState {
        match &self.open {
            OpenState::Opened(opened) => &opened.viewer,
            _ => unreachable!("viewer access requires an open document"),
        }
    }

    pub(crate) fn viewer_mut(&mut self) -> &mut ViewerState {
        match &mut self.open {
            OpenState::Opened(opened) => &mut opened.viewer,
            _ => unreachable!("viewer access requires an open document"),
        }
    }

    /// The open document (test observation).
    #[cfg(test)]
    pub(crate) fn opened_document(&self) -> Option<&OpenedDocument> {
        match &self.open {
            OpenState::Opened(opened) => Some(opened),
            _ => None,
        }
    }

    /// Replaces the open document wholesale (UI-test injection: multi-page
    /// documents without going through a worker-backed open).
    #[cfg(test)]
    pub(crate) fn open_test_document(
        &mut self,
        document: mark_core::Document,
        cx: &mut Context<Self>,
    ) {
        let page_sizes = document
            .pages()
            .iter()
            .map(|page| Vec2::new(page.width(), page.height()))
            .collect();
        self.open = OpenState::Opened(Box::new(OpenedDocument {
            session: DocumentSession::new(document),
            pdf: None,
            viewer: ViewerState::new_pdf(page_sizes),
            selected_object: None,
            press_pointer: None,
            gesture: None,
        }));
        cx.notify();
    }

    /// Opens the native file dialog and loads whatever the user picks.
    fn open_with_dialog(&mut self, cx: &mut Context<Self>) {
        let dialogs = platform::NativeFileDialogs;
        cx.spawn(async move |this, cx| {
            let Some(path) = platform::FilePicker::pick_open_document(&dialogs).await else {
                return;
            };
            this.update(cx, |app, cx| app.open_document_unfocused(path, cx))
                .ok();
        })
        .detach();
    }

    /// Opens `path` as a document, guarding a dirty document first:
    /// dropping unexported placements is a decision the user makes in the
    /// discard dialog, not a side effect of opening something else
    /// (plan.md §16). The dialog takes the keyboard.
    pub(crate) fn open_document(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.request_open(path, cx) {
            self.confirm_focus.focus(window, cx);
        }
    }

    /// [`Self::open_document`] for callers without a window (the file
    /// dialog's async return): the dialog is mouse-operable, and Escape
    /// still cancels it through the app-scope fallback.
    pub(crate) fn open_document_unfocused(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.request_open(path, cx);
    }

    /// The shared dirty guard: `true` when the discard dialog opened.
    fn request_open(&mut self, path: PathBuf, cx: &mut Context<Self>) -> bool {
        if self.is_dirty() {
            self.confirm = Some(PendingAction::Open(path));
            cx.notify();
            true
        } else {
            self.open_path(path, cx);
            false
        }
    }

    /// Ctrl+Q / window close: quits immediately when clean, otherwise the
    /// discard dialog decides (plan.md §16).
    pub(crate) fn handle_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dirty() {
            self.confirm = Some(PendingAction::Quit);
            self.confirm_focus.focus(window, cx);
            cx.notify();
            return;
        }
        cx.quit();
    }

    /// Whether the open document has unexported changes.
    fn is_dirty(&self) -> bool {
        match &self.open {
            OpenState::Opened(opened) => opened.session.is_dirty(),
            _ => false,
        }
    }

    /// Cancel: keep the document exactly as it is.
    fn cancel_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.confirm.take().is_some() {
            Self::reclaim_focus(self, window, cx);
            cx.notify();
        }
        true
    }

    /// Discard: drop the changes and run the action they were blocking.
    fn discard_and_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(pending) = self.confirm.take() {
            Self::reclaim_focus(self, window, cx);
            self.perform_pending(pending, cx);
            cx.notify();
        }
        true
    }

    /// Export…: close the dialog and run the export flow; the pending
    /// action resumes when the export completes. Cancelling the export
    /// dialog or a failed export leaves the document open and clean of
    /// prompts — the user retries whatever they were doing.
    fn export_then_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = self.confirm.take() {
            Self::reclaim_focus(self, window, cx);
            self.resume_after_export = Some(pending);
            cx.notify();
            self.handle_export(window, cx);
        }
    }

    /// Runs an action that was waiting on the discard dialog.
    fn perform_pending(&mut self, pending: PendingAction, cx: &mut Context<Self>) {
        match pending {
            PendingAction::Open(path) => self.open_path(path, cx),
            PendingAction::Quit => cx.quit(),
        }
    }

    /// The window-close hook: `true` lets the window close.
    pub(crate) fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.is_dirty() {
            self.confirm = Some(PendingAction::Quit);
            self.confirm_focus.focus(window, cx);
            cx.notify();
            return false;
        }
        true
    }

    /// Records a successfully opened document in the recents list and
    /// persists a snapshot of it off the UI thread (plan.md §18). A failed
    /// write only loses convenience data; the in-memory list stands.
    /// Paths are canonicalized so a relative CLI argument still opens from
    /// whatever directory the next launch uses.
    fn record_recent(&mut self, path: &Path, cx: &mut Context<Self>) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent.record(&canonical);
        let Some(file) = self.recent_path.clone() else {
            return;
        };
        let snapshot = self.recent.clone();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move { snapshot.save(&file) })
                .await;
            if saved.is_err() {
                this.update(cx, |_, _| {
                    tracing::warn!("the recent documents list could not be saved");
                })
                .ok();
            }
        })
        .detach();
    }

    /// Loads `path` as a document, images and PDFs alike, off the UI thread.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // A fresh document resets the previous document's export feedback.
        self.export = None;
        if is_pdf(&path) {
            self.open_pdf(path, cx);
        } else {
            self.open_image(path, cx);
        }
    }

    // ----- signature/stamp assets (plan.md §12) ---------------------------------

    /// Picks an image, imports it into the library as `kind`, and refreshes
    /// the panel. Importing re-encodes to a normalized RGBA PNG inside the
    /// app data directory (plan.md §9.3) — the source file is never touched.
    pub(crate) fn import_asset(&mut self, kind: AssetKind, cx: &mut Context<Self>) {
        let dialogs = platform::NativeFileDialogs;
        cx.spawn(async move |this, cx| {
            let Some(path) = platform::FilePicker::pick_open_image(&dialogs).await else {
                return;
            };
            this.update(cx, |app, cx| {
                let root = app.library.root().to_path_buf();
                cx.spawn(async move |this, cx| {
                    let imported = cx
                        .background_executor()
                        .spawn(async move {
                            AssetLibrary::open(&root).and_then(|mut library| {
                                library.import(&path, kind).map(|_| library)
                            })
                        })
                        .await;
                    this.update(cx, |app, cx| {
                        match imported {
                            Ok(library) => {
                                // The imported library includes the new
                                // asset; adopt it wholesale.
                                app.library = library;
                                app.library_notice = None;
                                app.load_asset_images(cx);
                            }
                            Err(_) => {
                                app.library_notice = Some(
                                    "Could not import the image. It may be damaged or unsupported."
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }

    /// Places `asset` centered on the current page at ~25% page width,
    /// selected and ready to drag (plan.md §12) — one undoable command.
    pub(crate) fn place_asset(&mut self, id: AssetId, cx: &mut Context<Self>) {
        let Some(asset) = self.library.asset(id) else {
            return;
        };
        let aspect = asset.aspect();
        if let OpenState::Opened(opened) = &mut self.open {
            let page_index = opened.viewer.current_page() as usize;
            if let Some(page) = opened.session.document().pages().get(page_index) {
                let object = DocumentObject::image(ImageObject::centered_on_page(page, id, aspect));
                let object_id = object.id();
                opened
                    .session
                    .execute(Box::new(AddObject::new(page.id(), object)));
                opened.selected_object = Some(object_id);
                cx.notify();
            }
        }
    }

    /// Escape: cancels any in-progress gesture, then clears the object
    /// selection (plan.md §15). An open discard dialog outranks both —
    /// this is the fallback for when it could not take focus (opened from
    /// the async file-dialog return).
    fn handle_clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() {
            self.cancel_confirm(window, cx);
            return;
        }
        let mut changed = false;
        if let OpenState::Opened(opened) = &mut self.open {
            changed = opened.gesture.take().is_some();
            changed |= opened.selected_object.take().is_some();
        }
        if changed {
            Self::reclaim_focus(self, window, cx);
            cx.notify();
        }
    }

    /// Selection gone → toolbar unmounted: return keyboard focus to the app
    /// scope so shortcuts keep dispatching (a focused toolbar control dies
    /// with its toolbar).
    fn reclaim_focus(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        window.focus(&self.focus_handle.clone(), cx);
    }

    // ----- object manipulation (plan.md §13, §14) --------------------------------

    /// Press inside `object`: selects it and records the press position as
    /// the anchor for a drag that may follow.
    pub(crate) fn object_press(&mut self, id: ObjectId, pointer: Vec2, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open {
            opened.press_pointer = Some(pointer);
            if opened.selected_object != Some(id) {
                opened.selected_object = Some(id);
            }
            cx.notify();
        }
    }

    /// Press inside a resize handle: keeps the selection (the press must
    /// not bubble to the canvas, which would clear it) and anchors a
    /// possible resize drag at the press position.
    pub(crate) fn handle_press(&mut self, pointer: Vec2) {
        if let OpenState::Opened(opened) = &mut self.open {
            opened.press_pointer = Some(pointer);
        }
    }

    /// The anchor a gesture starts from: the recorded press, falling back
    /// to the drag-activation position.
    fn gesture_anchor(opened: &OpenedDocument, drag_start: Vec2) -> Vec2 {
        opened.press_pointer.unwrap_or(drag_start)
    }

    /// A move drag started on `object` (drag-activation pointer position).
    pub(crate) fn begin_move(&mut self, object: ObjectId, pointer: Vec2) {
        if let OpenState::Opened(opened) = &mut self.open {
            let Some(mark_core::ObjectKind::Image(data)) = opened
                .session
                .document()
                .find_object(object)
                .map(|object| object.kind().clone())
            else {
                return;
            };
            let anchor = Self::gesture_anchor(opened, pointer);
            opened.gesture = Some(crate::manipulation::Gesture::begin_move(
                object,
                Vec2::new(data.x, data.y),
                Vec2::new(data.width, data.height),
                anchor,
            ));
        }
    }

    /// A resize drag started on `object`'s `corner` handle.
    pub(crate) fn begin_resize(
        &mut self,
        object: ObjectId,
        corner: crate::manipulation::Corner,
        aspect_lock: bool,
        pointer: Vec2,
    ) {
        if let OpenState::Opened(opened) = &mut self.open {
            let transform = opened.viewer.transform().unwrap_or_default();
            let Some(mark_core::ObjectKind::Image(data)) = opened
                .session
                .document()
                .find_object(object)
                .map(|object| object.kind().clone())
            else {
                return;
            };
            let anchor = Self::gesture_anchor(opened, pointer);
            opened.gesture = Some(crate::manipulation::Gesture::begin_resize(
                object,
                Rect {
                    x: data.x,
                    y: data.y,
                    width: data.width,
                    height: data.height,
                },
                corner,
                aspect_lock,
                anchor,
                transform,
            ));
        }
    }

    /// Canvas click on empty space (no object/handle consumed it): clear
    /// the selection. A pending gesture is untouched — its release still
    /// commits.
    pub(crate) fn clear_selection_if_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open
            && opened.selected_object.take().is_some()
        {
            Self::reclaim_focus(self, window, cx);
            cx.notify();
        }
    }

    /// Pointer moved during an active gesture: refresh the live preview.
    pub(crate) fn move_gesture(&mut self, pointer: Vec2, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open
            && let Some(gesture) = opened.gesture.as_mut()
        {
            let transform = opened.viewer.transform().unwrap_or_default();
            gesture.update_pointer(pointer, transform);
            cx.notify();
        }
    }

    /// Pointer released: commit the gesture as one undoable command
    /// (plan.md §14 — never one undo step per pointer movement).
    pub(crate) fn end_gesture(&mut self, cx: &mut Context<Self>) {
        let gesture = match &mut self.open {
            OpenState::Opened(opened) => {
                opened.press_pointer = None;
                opened.gesture.take()
            }
            _ => None,
        };
        let Some(gesture) = gesture else {
            return;
        };
        if let Some(command) = gesture.commit()
            && let OpenState::Opened(opened) = &mut self.open
        {
            opened.session.execute(command);
        }
        cx.notify();
    }

    /// Delete/Backspace: removes the selected overlay object — never page
    /// content (plan.md §13.2).
    fn handle_delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open
            && let Some(id) = opened.selected_object.take()
            && opened.session.document().find_object(id).is_some()
        {
            opened.session.execute(Box::new(DeleteObject::new(id)));
            Self::reclaim_focus(self, window, cx);
            cx.notify();
        }
    }

    // ----- multi-object / multi-page workflow (plan.md §13.1, §13.2) ---------

    /// The selected image object, cloned out of the document.
    fn selected_image(&self) -> Option<(ObjectId, ImageObject)> {
        let opened = match &self.open {
            OpenState::Opened(opened) => opened,
            _ => return None,
        };
        let id = opened.selected_object?;
        #[allow(irrefutable_let_patterns)] // Text/Shape variants come later (plan.md §7)
        let ObjectKind::Image(data) = opened.session.document().find_object(id)?.kind() else {
            return None;
        };
        Some((id, data.clone()))
    }

    /// Ctrl+C: copies the selected object; pastes land on the current page
    /// offset from the copy's own position (plan.md §13.2).
    fn handle_copy(&mut self, _cx: &mut Context<Self>) {
        if let Some((_, data)) = self.selected_image() {
            self.clipboard = Some(data);
            self.paste_count = 0;
        }
    }

    /// Ctrl+V: pastes the clipboard onto the current page, one step further
    /// out per consecutive paste, always fully on the page.
    fn handle_paste(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.clipboard.clone() else {
            return;
        };
        if let OpenState::Opened(opened) = &mut self.open {
            let page_index = opened.viewer.current_page() as usize;
            let Some(page) = opened.session.document().pages().get(page_index) else {
                return;
            };
            self.paste_count += 1;
            let step = PASTE_STEP * self.paste_count as f32;
            let mut pasted = data;
            pasted.x += step;
            pasted.y += step;
            let page_size = Vec2::new(page.width(), page.height());
            let clamped = crate::manipulation::clamped_into_page(
                Rect {
                    x: pasted.x,
                    y: pasted.y,
                    width: pasted.width,
                    height: pasted.height,
                },
                page_size,
            );
            (pasted.x, pasted.y) = (clamped.x, clamped.y);
            let object = DocumentObject::image(pasted);
            let object_id = object.id();
            opened
                .session
                .execute(Box::new(AddObject::new(page.id(), object)));
            opened.selected_object = Some(object_id);
            cx.notify();
        }
    }

    /// Ctrl+D: duplicates the selected object on its own page, offset so it
    /// lands beside the original, and selects the duplicate.
    fn handle_duplicate(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = self.selected_image() else {
            return;
        };
        if let OpenState::Opened(opened) = &mut self.open
            && let Some(source) = opened.session.document().find_object(id).cloned()
        {
            let command =
                DuplicateObject::new(id, source, Vec2::new(DUPLICATE_OFFSET, DUPLICATE_OFFSET));
            let duplicate_id = command.duplicate.id();
            opened.session.execute(Box::new(command));
            opened.selected_object = Some(duplicate_id);
            cx.notify();
        }
    }

    /// "Duplicate to page…": places a copy of the selected object on
    /// `page_index`, clamped fully onto that page; the source stays
    /// selected on the current page (plan.md §13.1).
    pub(crate) fn duplicate_to_page(&mut self, page_index: usize, cx: &mut Context<Self>) {
        let Some((_, mut data)) = self.selected_image() else {
            return;
        };
        if let OpenState::Opened(opened) = &mut self.open {
            let current = opened.viewer.current_page() as usize;
            let Some(target) = opened.session.document().pages().get(page_index) else {
                return;
            };
            // Same-page duplication is Ctrl+D's job.
            if page_index == current {
                return;
            }
            let page_size = Vec2::new(target.width(), target.height());
            let clamped = crate::manipulation::clamped_into_page(
                Rect {
                    x: data.x,
                    y: data.y,
                    width: data.width,
                    height: data.height,
                },
                page_size,
            );
            (data.x, data.y) = (clamped.x, clamped.y);
            let object = DocumentObject::image(data);
            opened
                .session
                .execute(Box::new(DuplicateToPage::new(object, target.id())));
            // The copy is invisible from the current page: say where it
            // landed (the Phase 8 open point).
            self.push_toast(
                format!("Duplicated to page {}", page_index + 1),
                ToastKind::Info,
                cx,
            );
            tracing::info!(page = page_index + 1, "Duplicated to page");
            cx.notify();
        }
    }

    /// Toolbar size step: scales the selected object about its center,
    /// aspect preserved, clamped to the page (plan.md §13).
    fn handle_resize_selected(&mut self, factor: f32, cx: &mut Context<Self>) {
        let Some((id, data)) = self.selected_image() else {
            return;
        };
        if let OpenState::Opened(opened) = &mut self.open {
            let page_index = opened.viewer.current_page() as usize;
            let Some(page) = opened.session.document().pages().get(page_index) else {
                return;
            };
            let from = Rect {
                x: data.x,
                y: data.y,
                width: data.width,
                height: data.height,
            };
            let to = crate::manipulation::scaled_about_center(
                from,
                factor,
                Vec2::new(page.width(), page.height()),
            );
            // A no-op step (already at page size) commits nothing.
            if (to.width - from.width).abs() < 1e-3 && (to.height - from.height).abs() < 1e-3 {
                return;
            }
            opened
                .session
                .execute(Box::new(ResizeObject::new(id, from, to)));
            cx.notify();
        }
    }

    fn handle_undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open
            && opened.session.undo()
        {
            let had_selection = opened.selected_object.is_some();
            Self::validate_selection(opened);
            if had_selection && opened.selected_object.is_none() {
                Self::reclaim_focus(self, window, cx);
            }
            cx.notify();
        }
    }

    fn handle_redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let OpenState::Opened(opened) = &mut self.open
            && opened.session.redo()
        {
            let had_selection = opened.selected_object.is_some();
            Self::validate_selection(opened);
            if had_selection && opened.selected_object.is_none() {
                Self::reclaim_focus(self, window, cx);
            }
            cx.notify();
        }
    }

    /// Drops a selection that no longer points at a live object (undo of a
    /// placement, redo of a delete, …).
    fn validate_selection(opened: &mut OpenedDocument) {
        if let Some(id) = opened.selected_object
            && opened.session.document().find_object(id).is_none()
        {
            opened.selected_object = None;
        }
    }

    /// Releases the previous document's worker-side resources, if any.
    fn close_previous(&mut self) {
        if let OpenState::Opened(opened) = &self.open
            && let Some(handle) = opened.pdf
        {
            self.pdf.close(handle);
        }
    }

    fn open_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name: SharedString = file_name(&path).into();
        self.open = OpenState::Opening { name: name.clone() };
        cx.notify();

        cx.spawn(async move |this, cx| {
            // The loader consumes its own copy; `path` stays for the
            // recents record on success.
            let load_path = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { mark_image::ImageDocument::load(&load_path) })
                .await;
            this.update(cx, |app, cx| {
                app.open = match loaded {
                    Ok(image_document) => {
                        let (document, rgba) = image_document.into_parts();
                        let page = document
                            .pages()
                            .first()
                            .map(|page| Vec2::new(page.width(), page.height()))
                            .unwrap_or_else(|| Vec2::new(612., 792.));
                        let mut viewer = ViewerState::new_image(page);
                        viewer.seed_page_image(
                            0,
                            rgba.width(),
                            Arc::new(canvas::render_image(&rgba)),
                        );
                        tracing::info!(pages = document.page_count(), "Opened image document");
                        app.record_recent(&path, cx);
                        OpenState::Opened(Box::new(OpenedDocument {
                            session: DocumentSession::new(document),
                            pdf: None,
                            viewer,
                            selected_object: None,
                            press_pointer: None,
                            gesture: None,
                        }))
                    }
                    Err(_) => OpenState::Failed { name },
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Loads a PDF on the PDFium worker (plan.md §6.4); page renders and
    /// thumbnails follow from `refresh_view` once the viewport is measured.
    fn open_pdf(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name: SharedString = file_name(&path).into();
        self.close_previous();
        self.open = OpenState::Opening { name: name.clone() };
        cx.notify();

        let worker = self.pdf.clone();
        cx.spawn(async move |this, cx| {
            // Load: document metadata plus the worker-side handle. The app
            // only distinguishes success from failure here; friendly error
            // copy arrives with the error surfaces (Phase 10, plan.md §17).
            let loaded: Option<mark_pdf::LoadedPdf> = worker
                .load(path.clone())
                .await
                .inspect_err(|error| {
                    tracing::warn!(%error, "the PDF worker became unavailable");
                })
                .ok()
                .and_then(|result| {
                    result
                        .inspect_err(|error| {
                            tracing::warn!(%error, path = %path.display(), "could not load the PDF");
                        })
                        .ok()
                });

            this.update(cx, |app, cx| {
                match loaded {
                    Some(loaded) => {
                        let handle = loaded.handle;
                        let page_sizes = loaded
                            .pages
                            .iter()
                            .map(|geometry| geometry.display_size())
                            .collect();
                        tracing::info!(pages = loaded.document.page_count(), "Opened PDF document");
                        app.record_recent(&path, cx);
                        app.open = OpenState::Opened(Box::new(OpenedDocument {
                            session: DocumentSession::new(loaded.document),
                            pdf: Some(handle),
                            viewer: ViewerState::new_pdf(page_sizes),
                            selected_object: None,
                            press_pointer: None,
                            gesture: None,
                        }));
                        cx.notify();
                        // The viewport is still unknown; the canvas probe
                        // triggers the first render request after layout.
                        app.refresh_view(cx);
                    }
                    None => {
                        app.open = OpenState::Failed { name };
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    // ----- navigation and zoom ------------------------------------------------

    pub(crate) fn go_to_page(&mut self, page: u32, cx: &mut Context<Self>) {
        self.navigate_with(cx, |viewer| viewer.navigate_to(page))
    }

    fn handle_next_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::next_page);
    }

    fn handle_previous_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::previous_page);
    }

    fn handle_first_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::first_page);
    }

    fn handle_last_page(&mut self, cx: &mut Context<Self>) {
        self.navigate_with(cx, ViewerState::last_page);
    }

    fn navigate_with(
        &mut self,
        cx: &mut Context<Self>,
        navigate: impl FnOnce(&mut ViewerState) -> bool,
    ) {
        let mut changed = false;
        if let OpenState::Opened(opened) = &mut self.open {
            changed = navigate(&mut opened.viewer);
            if changed {
                opened
                    .viewer
                    .prioritize_thumbnail(opened.viewer.current_page());
                cx.notify();
            }
        }
        if changed {
            self.refresh_view(cx);
        }
    }

    fn handle_zoom_in(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_in);
    }

    fn handle_zoom_out(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_out);
    }

    fn handle_zoom_fit(&mut self, cx: &mut Context<Self>) {
        self.zoom_with(cx, ViewerState::zoom_fit);
    }

    fn zoom_with(&mut self, cx: &mut Context<Self>, zoom: impl FnOnce(&mut ViewerState)) {
        if let OpenState::Opened(opened) = &mut self.open {
            zoom(&mut opened.viewer);
            cx.notify();
        }
        self.refresh_view(cx);
    }

    // ----- render request loop (plan.md §11) -------------------------------------

    /// Issues the requests the current view needs: the current page's
    /// bitmap at viewport resolution, then queued thumbnails.
    pub(crate) fn refresh_view(&mut self, cx: &mut Context<Self>) {
        let wanted = match &self.open {
            OpenState::Opened(opened) => opened.pdf.zip(opened.viewer.wanted_render()),
            _ => None,
        };
        if let Some((handle, (page, width))) = wanted {
            if let OpenState::Opened(opened) = &mut self.open {
                opened.viewer.mark_render_requested(page, width);
            }
            let worker = self.pdf.clone();
            cx.spawn(async move |this, cx| {
                let rendered = worker
                    .render_page(handle, page, width)
                    .await
                    .ok()
                    .and_then(|result| result.ok());
                this.update(cx, |app, cx| match rendered {
                    Some(rendered) => app.record_page_render(handle, page, width, rendered, cx),
                    None => app.forget_page_render(handle, page, width, cx),
                })
                .ok();
            })
            .detach();
        }
        self.pump_thumbnails(cx);
    }

    /// A page render reply arrived: cache it, show it if current, and see
    /// what the view wants next.
    fn record_page_render(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        width: u32,
        rendered: RenderedPage,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.record_render(
                page,
                width,
                Arc::new(canvas::render_image(&rendered.rgba)),
            );
            true
        } else {
            false
        };
        if applied {
            cx.notify();
            self.refresh_view(cx);
        }
    }

    /// A page render failed or belongs to a closed document: drop the
    /// pending slot so a fresh request can be issued.
    fn forget_page_render(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        width: u32,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.clear_pending_render(page, width);
            true
        } else {
            false
        };
        if applied {
            self.refresh_view(cx);
        }
    }

    /// Issues the next queued thumbnail request, one at a time, never
    /// ahead of the current page's render (plan.md §11.2).
    fn pump_thumbnails(&mut self, cx: &mut Context<Self>) {
        loop {
            let next = match &mut self.open {
                OpenState::Opened(opened) => opened.pdf.zip(opened.viewer.next_thumbnail_request()),
                _ => None,
            };
            let Some((handle, page)) = next else {
                break;
            };
            let worker = self.pdf.clone();
            cx.spawn(async move |this, cx| {
                let rendered = worker
                    .render_page(handle, page, crate::viewer::THUMBNAIL_RENDER_WIDTH)
                    .await
                    .ok()
                    .and_then(|result| result.ok());
                this.update(cx, |app, cx| {
                    if let Some(rendered) = rendered {
                        app.record_thumbnail(handle, page, rendered, cx);
                    } else {
                        app.forget_thumbnail(handle, page, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn record_thumbnail(
        &mut self,
        handle: PdfDocumentHandle,
        page: u32,
        rendered: RenderedPage,
        cx: &mut Context<Self>,
    ) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened
                .viewer
                .record_thumbnail(page, Arc::new(canvas::render_image(&rendered.rgba)));
            true
        } else {
            false
        };
        if applied {
            cx.notify();
            self.refresh_view(cx);
        }
    }

    fn forget_thumbnail(&mut self, handle: PdfDocumentHandle, page: u32, cx: &mut Context<Self>) {
        let applied = if let OpenState::Opened(opened) = &mut self.open
            && opened.pdf == Some(handle)
        {
            opened.viewer.clear_thumbnail_in_flight(page);
            true
        } else {
            false
        };
        if applied {
            self.refresh_view(cx);
        }
    }

    // ----- export (plan.md §16) ---------------------------------------------------

    /// Ctrl+S / header button: asks where to save (default
    /// `<stem>-signed.<ext>` next to the original, never the original
    /// itself) and exports the open document.
    fn handle_export(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.opened_source() else {
            return;
        };
        if self.is_exporting() {
            return; // one job at a time
        }
        let extension = if is_pdf(&source) { "pdf" } else { "png" };
        let default = signed_destination(&source, extension);
        let dialogs = platform::NativeFileDialogs;
        cx.spawn(async move |this, cx| {
            // Cancelled dialog = cancelled export; nothing was set yet.
            // A pending discard-dialog action will not resume — the user
            // cancelled their way out of that decision.
            if let Some(destination) =
                platform::FilePicker::pick_save_export(&dialogs, default).await
            {
                this.update(cx, |app, cx| app.export_to(destination, cx))
                    .ok();
            } else {
                this.update(cx, |app, _| app.resume_after_export = None)
                    .ok();
            }
        })
        .detach();
    }

    /// The export core (dialog-free so tests drive it directly): snapshot
    /// the placed objects, then write the signed copy — PDFs through real
    /// PDFium image page objects on the worker thread (§16.1), image
    /// documents composited to PNG. The open document is only ever read,
    /// and the original file is never touched.
    pub(crate) fn export_to(&mut self, destination: PathBuf, cx: &mut Context<Self>) {
        let OpenState::Opened(opened) = &self.open else {
            return;
        };
        if self.is_exporting() {
            return;
        }
        let source = opened.session().source().path().to_path_buf();
        let total = opened.session().document().page_count() as u32;
        let specs: Vec<Vec<OverlaySpec>> = overlay_specs(opened.session().document());

        // Resolve asset image paths through the live library up front: a
        // placed asset that is no longer in the library fails the export
        // loudly instead of silently missing a signature (plan.md §17).
        let library_root = self.library.root().to_path_buf();
        let mut asset_paths: HashMap<AssetId, PathBuf> = HashMap::new();
        for OverlaySpec { asset, .. } in specs.iter().flatten() {
            if asset_paths.contains_key(asset) {
                continue;
            }
            match self.library.asset(*asset) {
                Some(asset_entry) => {
                    asset_paths.insert(*asset, asset_entry.image_path().to_path_buf());
                }
                None => {
                    self.export = Some(ExportStatus::Failed(
                        "A placed image is missing from the library.".into(),
                    ));
                    cx.notify();
                    return;
                }
            }
        }

        self.export = Some(ExportStatus::Running {
            done: 0,
            total: total.max(1),
        });
        cx.notify();

        if is_pdf(&source) {
            self.export_pdf(destination, source, specs, library_root, asset_paths, cx);
        } else {
            self.export_image(destination, source, specs, library_root, asset_paths, cx);
        }
    }

    /// PDF export: builds the worker job off the UI thread (reading the
    /// library's normalized PNGs), then tracks progress and the result.
    fn export_pdf(
        &mut self,
        destination: PathBuf,
        source: PathBuf,
        specs: Vec<Vec<OverlaySpec>>,
        library_root: PathBuf,
        asset_paths: HashMap<AssetId, PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let worker = self.pdf.clone();
        cx.spawn(async move |this, cx| {
            // The job owns the paths; keep copies for the finish report.
            let source_for_finish = source.clone();
            let destination_for_finish = destination.clone();
            // Read the assets' normalized PNGs off the UI thread.
            let job = cx
                .background_executor()
                .spawn(async move {
                    build_pdf_export(source, destination, &specs, &library_root, &asset_paths)
                })
                .await;
            match job {
                Ok(job) => {
                    let (mut progress, result) = worker.export(job);
                    // Progress events all precede the reply, so the stream
                    // ends exactly when the worker finishes.
                    while let Some(event) = progress.next().await {
                        this.update(cx, |app, cx| {
                            app.note_export_progress(event.done, event.total, cx);
                        })
                        .ok();
                    }
                    let outcome = match result.await {
                        Ok(Ok(())) => Ok(()),
                        _ => Err("The PDF could not be exported."),
                    };
                    this.update(cx, |app, cx| {
                        app.finish_export(outcome, &source_for_finish, destination_for_finish, cx)
                    })
                    .ok();
                }
                Err(message) => {
                    this.update(cx, |app, cx| {
                        app.finish_export(
                            Err(message),
                            &source_for_finish,
                            destination_for_finish,
                            cx,
                        )
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    /// Image export: reloads the source bitmap, composites the placed
    /// objects (plan.md §16 image rule), and writes PNG — all off the UI
    /// thread.
    fn export_image(
        &mut self,
        destination: PathBuf,
        source: PathBuf,
        specs: Vec<Vec<OverlaySpec>>,
        library_root: PathBuf,
        asset_paths: HashMap<AssetId, PathBuf>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            // The job owns the paths; keep copies for the finish report.
            let source_for_finish = source.clone();
            let destination_for_finish = destination.clone();
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    export_image_file(&source, &destination, &specs, &library_root, &asset_paths)
                })
                .await;
            this.update(cx, |app, cx| {
                app.finish_export(outcome, &source_for_finish, destination_for_finish, cx)
            })
            .ok();
        })
        .detach();
    }

    /// A progress event arrived from the running export.
    fn note_export_progress(&mut self, done: u32, total: u32, cx: &mut Context<Self>) {
        if matches!(self.export, Some(ExportStatus::Running { .. })) {
            self.export = Some(ExportStatus::Running {
                done,
                total: total.max(1),
            });
            cx.notify();
        }
    }

    /// The export finished: record the outcome and mark the originating
    /// session saved (plan.md §16 dirty state) — and resume whatever the
    /// export was blocking, when it came from the discard dialog.
    fn finish_export(
        &mut self,
        outcome: Result<(), &str>,
        source: &Path,
        destination: PathBuf,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(()) => {
                // Only the document that was exported (the user may have
                // opened another file while the worker wrote).
                if let OpenState::Opened(opened) = &mut self.open
                    && opened.session().source().path() == source
                {
                    opened.session.mark_saved();
                }
                let name: SharedString = file_name(&destination).into();
                tracing::info!(destination = %name, "Export completed");
                self.push_toast(format!("Exported {name}"), ToastKind::Success, cx);
                self.export = Some(ExportStatus::Done { name });
                if let Some(pending) = self.resume_after_export.take() {
                    self.perform_pending(pending, cx);
                }
            }
            Err(message) => {
                // The pending action stays un-run: the document is still
                // there, the user decides again.
                self.resume_after_export = None;
                self.export = Some(ExportStatus::Failed(message.into()));
            }
        }
        cx.notify();
    }

    fn is_exporting(&self) -> bool {
        matches!(self.export, Some(ExportStatus::Running { .. }))
    }

    // ----- toasts (plan.md §10, §17) ---------------------------------------------

    /// Shows a transient notice; auto-dismisses after [`TOAST_TIMEOUT`].
    fn push_toast(
        &mut self,
        message: impl Into<SharedString>,
        kind: ToastKind,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(
            id,
            ToastNotice {
                message: message.into(),
                kind,
            },
            ToastOptions {
                timeout: Some(TOAST_TIMEOUT),
            },
            Instant::now(),
        );
        if self.toast_clock.is_none() {
            self.toast_clock = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TOAST_TICK).await;
                    let Ok(continue_clock) = this.update(cx, |app, cx| {
                        let advance = app.toasts.advance(Instant::now(), false);
                        if advance.changed {
                            cx.notify();
                        }
                        !app.toasts.is_empty()
                    }) else {
                        break;
                    };
                    if !continue_clock {
                        break;
                    }
                }
                // The clock restarts on the next push.
                this.update(cx, |app, _| app.toast_clock = None).ok();
            }));
        }
        cx.notify();
    }

    // ----- window title (plan.md §16) ---------------------------------------------

    /// The title text for the current state: the open document's name with
    /// a `*` marker while it has unexported changes, else the app name.
    fn window_title_text(&self) -> String {
        match &self.open {
            OpenState::Opened(opened) => {
                let name = opened
                    .session()
                    .source()
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Document");
                if opened.session.is_dirty() {
                    format!("{name} *")
                } else {
                    name.to_owned()
                }
            }
            _ => "Mark".to_owned(),
        }
    }

    /// Pushes the title to the platform when it changed. Called from
    /// `render`: every dirty-state transition notifies (commands, undo,
    /// redo, export), so the next frame is always the right moment, and
    /// the platform call itself is change-guarded.
    fn sync_window_title(&mut self, window: &mut Window) {
        let title = self.window_title_text();
        if title != self.window_title {
            self.window_title = title;
            window.set_window_title(&self.window_title);
        }
    }

    /// The open document's source path, if any.
    fn opened_source(&self) -> Option<PathBuf> {
        match &self.open {
            OpenState::Opened(opened) => Some(opened.session().source().path().to_path_buf()),
            _ => None,
        }
    }

    /// Export feedback state (test observation).
    #[cfg(test)]
    pub(crate) fn export_status(&self) -> Option<&ExportStatus> {
        self.export.as_ref()
    }

    /// Mounted toast messages, oldest first (test observation).
    #[cfg(test)]
    pub(crate) fn toast_messages(&self) -> Vec<String> {
        self.toasts
            .iter()
            .map(|(_, notice, _)| notice.message.to_string())
            .collect()
    }

    /// The pending action shown in the discard dialog, if any (test
    /// observation).
    #[cfg(test)]
    pub(crate) fn confirm_pending(&self) -> Option<&PendingAction> {
        self.confirm.as_ref()
    }

    /// The computed window title (test observation; the platform title is
    /// not readable back in headless tests).
    #[cfg(test)]
    pub(crate) fn title_text(&self) -> &str {
        &self.window_title
    }

    fn header_title(&self) -> Option<&str> {
        match &self.open {
            OpenState::Opened(opened) => opened.session().source().path().file_name()?.to_str(),
            _ => None,
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    fn open_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "⌘O"
        } else {
            "Ctrl+O"
        }
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// The document's placed objects as per-page overlay specs, in page order
/// — the export snapshot, taken once before any bytes move.
fn overlay_specs(document: &mark_core::Document) -> Vec<Vec<OverlaySpec>> {
    document
        .pages()
        .iter()
        .map(|page| {
            page.objects()
                .iter()
                .filter_map(|object| {
                    #[allow(irrefutable_let_patterns)]
                    // Text/Shape variants come later (plan.md §7)
                    let ObjectKind::Image(data) = object.kind() else {
                        return None;
                    };
                    Some(OverlaySpec {
                        rect: Rect {
                            x: data.x,
                            y: data.y,
                            width: data.width,
                            height: data.height,
                        },
                        asset: data.asset_id,
                        opacity: data.opacity,
                    })
                })
                .collect()
        })
        .collect()
}

/// Assembles the worker job: every placed object's asset PNG read once,
/// mapped to its display-space page rect (plan.md §16.1).
///
/// Runs off the UI thread; a placed asset whose normalized PNG vanished
/// from disk fails the export loudly (plan.md §17) instead of silently
/// writing a signed copy that misses a signature.
fn build_pdf_export(
    source: PathBuf,
    destination: PathBuf,
    specs: &[Vec<OverlaySpec>],
    library_root: &Path,
    asset_paths: &HashMap<AssetId, PathBuf>,
) -> Result<PdfExport, &'static str> {
    let mut pngs: HashMap<AssetId, Vec<u8>> = HashMap::new();
    for (asset, relative) in asset_paths {
        let bytes = std::fs::read(library_root.join(relative))
            .map_err(|_| "A placed image could not be read from the library.")?;
        pngs.insert(*asset, bytes);
    }
    let pages = specs
        .iter()
        .map(|page| {
            page.iter()
                .map(|spec| ImageOverlay {
                    rect: spec.rect,
                    png: pngs.get(&spec.asset).expect("spec asset was read").clone(),
                })
                .collect()
        })
        .collect();
    Ok(PdfExport {
        source,
        destination,
        pages,
    })
}

/// Writes the signed PNG for an image document: the reloaded source bitmap
/// with every placed object composited at its page rect (image pages are
/// 1 px = 1 pt, plan.md §8, so point coordinates map 1:1 to pixels).
fn export_image_file(
    source: &Path,
    destination: &Path,
    specs: &[Vec<OverlaySpec>],
    library_root: &Path,
    asset_paths: &HashMap<AssetId, PathBuf>,
) -> Result<(), &'static str> {
    let (_, base) = mark_image::ImageDocument::load(source)
        .map_err(|_| "The document could not be read again for export.")?
        .into_parts();
    // Decode each placed asset once, before any borrows of the map.
    let mut bitmaps: HashMap<AssetId, image::RgbaImage> = HashMap::new();
    for spec in specs.first().map(|page| page.as_slice()).unwrap_or(&[]) {
        if bitmaps.contains_key(&spec.asset) {
            continue;
        }
        let path = library_root.join(asset_paths.get(&spec.asset).expect("resolved"));
        let decoded = mark_image::ImageDocument::load(&path)
            .map_err(|_| "A placed image could not be read from the library.")?
            .into_parts()
            .1;
        bitmaps.insert(spec.asset, decoded);
    }
    let overlays: Vec<PngOverlay<'_>> = specs
        .first()
        .map(|page| {
            page.iter()
                .map(|spec| PngOverlay {
                    rect: spec.rect,
                    image: &bitmaps[&spec.asset],
                    opacity: spec.opacity,
                })
                .collect()
        })
        .unwrap_or_default();
    let composed = compose_png_page(&base, &overlays);
    composed
        .save(destination)
        .map_err(|_| "The exported file could not be written.")
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
}

impl Render for MarkApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        self.sync_window_title(window);
        let zoom_percent = match &self.open {
            OpenState::Opened(opened) => opened.viewer.zoom_percent(),
            _ => None,
        };
        // Owned snapshots built under immutable borrows, so the mutable
        // `&mut self.open` match below never conflicts with library reads.
        let library = assets::LibrarySnapshot {
            assets: self.library.assets().to_vec(),
            images: self.asset_images.clone(),
            notice: self.library_notice.clone(),
        };
        let can_export = matches!(self.open, OpenState::Opened(_));
        let exporting = self.is_exporting();
        let export_status = self.export.clone();
        // Recents ride beside the empty workspace only: with a document
        // open the list is unreachable (and unneeded).
        let recent: Vec<PathBuf> = if matches!(self.open, OpenState::Empty) {
            self.recent
                .paths()
                .iter()
                .filter(|path| path.is_file())
                .take(5)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let confirm = self.confirm.is_some();
        let toasts: Vec<(u64, ToastNotice)> = self
            .toasts
            .iter()
            .map(|(id, notice, _)| (*id, notice.clone()))
            .collect();

        focus_scope("mark")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &OpenDocument, _window, cx| {
                this.open_with_dialog(cx);
            }))
            .on_action(cx.listener(|this, _: &QuitAction, window, cx| {
                this.handle_quit(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &NextPageAction, _window, cx| this.handle_next_page(cx)),
            )
            .on_action(cx.listener(|this, _: &PreviousPageAction, _window, cx| {
                this.handle_previous_page(cx);
            }))
            .on_action(cx.listener(|this, _: &FirstPageAction, _window, cx| {
                this.handle_first_page(cx);
            }))
            .on_action(
                cx.listener(|this, _: &LastPageAction, _window, cx| this.handle_last_page(cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomInAction, _window, cx| this.handle_zoom_in(cx)))
            .on_action(cx.listener(|this, _: &ZoomOutAction, _window, cx| this.handle_zoom_out(cx)))
            .on_action(cx.listener(|this, _: &ZoomFitAction, _window, cx| this.handle_zoom_fit(cx)))
            .on_action(cx.listener(|this, _: &ClearSelectionAction, window, cx| {
                this.handle_clear_selection(window, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteSelectedAction, window, cx| {
                this.handle_delete_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &CopyAction, _window, cx| this.handle_copy(cx)))
            .on_action(cx.listener(|this, _: &PasteAction, _window, cx| this.handle_paste(cx)))
            .on_action(
                cx.listener(|this, _: &DuplicateAction, _window, cx| this.handle_duplicate(cx)),
            )
            .on_action(cx.listener(|this, _: &UndoAction, window, cx| this.handle_undo(window, cx)))
            .on_action(cx.listener(|this, _: &RedoAction, window, cx| this.handle_redo(window, cx)))
            .on_action(
                cx.listener(|this, _: &ExportAction, window, cx| this.handle_export(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.inset)
            .child(header(
                &theme,
                self.header_title(),
                zoom_percent,
                can_export,
                exporting,
                cx,
            ))
            .child(match &mut self.open {
                OpenState::Empty => empty_workspace(&theme, &recent, cx).into_any_element(),
                OpenState::Opening { name } => {
                    notice_workspace(&theme, IconName::FileText, format!("Opening {name}…"), None)
                        .into_any_element()
                }
                OpenState::Failed { name } => notice_workspace(
                    &theme,
                    IconName::FileX,
                    format!("Could not open {name}"),
                    Some("The file may be damaged or in an unsupported format."),
                )
                .into_any_element(),
                OpenState::Opened(opened) => workspace(&theme, opened, &library, window, cx),
            })
            .children(match &self.open {
                OpenState::Opened(opened) => Some(status_bar(
                    &theme,
                    opened.viewer.current_page(),
                    opened.viewer.page_count(),
                    opened.viewer.has_previous_page(),
                    opened.viewer.has_next_page(),
                    zoom_percent,
                    export_status.as_ref(),
                    cx,
                )),
                _ => None,
            })
            .children(confirm.then(|| {
                discard_dialog(
                    &theme,
                    self.header_title().unwrap_or("the document"),
                    &self.confirm_focus,
                    cx,
                )
            }))
            .children((!toasts.is_empty()).then(|| toast_area(&theme, &toasts, cx)))
    }
}

/// The discard-changes dialog (plan.md §16): Export… runs the export flow
/// and resumes the blocked action on success, Discard drops the changes,
/// Cancel keeps everything as it is. Escape cancels; the backdrop never
/// dismisses an explicit decision.
fn discard_dialog(
    theme: &Theme,
    name: &str,
    focus: &FocusHandle,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    let _ = theme;
    // Base dialog handlers return a "handled" bool; `cx.listener` cannot
    // produce one, so the entity update is explicit here.
    let discard = cx.weak_entity();
    let cancel = cx.weak_entity();
    gpui_omarchy::alert_dialog(focus, cx)
        .open(true)
        .on_ok(move |_, window, cx| {
            discard
                .update(cx, |app, cx| app.discard_and_continue(window, cx))
                .unwrap_or(true)
        })
        .on_cancel(move |_, window, cx| {
            cancel
                .update(cx, |app, cx| app.cancel_confirm(window, cx))
                .unwrap_or(true)
        })
        .popup(
            dialog_popup(cx)
                .child(dialog_title("Discard changes?", cx))
                .child(dialog_description(
                    format!("“{name}” has placements that have not been exported yet."),
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(rems(0.5))
                        .child(AlertDialogCancel::new().child(dialog_button(
                            "mark-confirm-cancel",
                            "Cancel",
                            ButtonVariant::Outline,
                            cx,
                        )))
                        .child(AlertDialogAction::new().child(dialog_button(
                            "mark-confirm-discard",
                            "Discard",
                            ButtonVariant::Danger,
                            cx,
                        )))
                        .child(
                            dialog_button(
                                "mark-confirm-export",
                                "Export…",
                                ButtonVariant::Primary,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.export_then_continue(window, cx),
                            )),
                        ),
                ),
        )
}

/// The toast stack, bottom-right (plan.md §10): asynchronous status that
/// requires no decision.
fn toast_area(
    theme: &Theme,
    toasts: &[(u64, ToastNotice)],
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    div()
        .id("mark-toast-area")
        .test_support()
        .absolute()
        .right(rems(0.875))
        .bottom(rems(2.5))
        .w(rems(20.))
        .flex()
        .flex_col()
        .gap(rems(0.5))
        .occlude()
        .children(toasts.iter().map(|(id, notice)| {
            gpui_omarchy::toast(format!("mark-toast-{id}"), cx).child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(0.625))
                    .child(
                        icon(notice.kind.icon())
                            .size(rems(1.))
                            .text_color(notice.kind.color(theme)),
                    )
                    .child(div().child(notice.message.clone())),
            )
        }))
}

/// The document workspace: thumbnail sidebar, canvas stage with placed
/// objects, and the assets panel (plan.md §10).
fn workspace(
    theme: &Theme,
    opened: &mut OpenedDocument,
    library: &assets::LibrarySnapshot,
    window: &mut Window,
    cx: &mut Context<MarkApp>,
) -> gpui_kit::AnyElement {
    let weak = cx.weak_entity();
    let page_index = opened.viewer.current_page() as usize;
    let page = opened.session.document().pages().get(page_index);
    let selected = opened.selected_object;
    // Placed objects of the current page, resolved to cached bitmaps: the
    // canvas draws page → objects → selection in one pass (plan.md §13).
    let gesture = opened.gesture.as_ref();
    let placed: Vec<canvas::PlacedObject> = page
        .map(|page| {
            page.objects()
                .iter()
                .map(|object| {
                    let mark_core::ObjectKind::Image(data) = object.kind();
                    // An in-progress gesture replaces the stored geometry
                    // with its live preview.
                    let live = gesture
                        .filter(|gesture| gesture.object == object.id())
                        .map(|gesture| gesture.live);
                    let (x, y, width, height) = match live {
                        Some(rect) => (rect.x, rect.y, rect.width, rect.height),
                        None => (data.x, data.y, data.width, data.height),
                    };
                    canvas::PlacedObject {
                        id: object.id(),
                        x,
                        y,
                        width,
                        height,
                        opacity: data.opacity,
                        image: library.images.get(&data.asset_id).cloned(),
                        selected: selected == Some(object.id()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let toolbar = selection_toolbar(theme, opened, library, cx);
    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .items_stretch()
        .overflow_hidden()
        .child(thumbnails::sidebar(theme, &opened.viewer, window, cx))
        .child(canvas::stage(
            theme,
            &weak,
            &mut opened.viewer,
            &placed,
            toolbar,
            cx,
        ))
        .child(assets::panel(theme, library, window, cx))
        .into_any_element()
}

/// The contextual toolbar for the selected object (plan.md §13): kind
/// label, size steps, duplicate, "To page…", delete — one command per
/// control. Rendered as a floating bar over the canvas; `None` whenever
/// nothing is selected.
fn selection_toolbar(
    theme: &Theme,
    opened: &OpenedDocument,
    library: &assets::LibrarySnapshot,
    cx: &mut Context<MarkApp>,
) -> Option<AnyElement> {
    let id = opened.selected_object?;
    #[allow(irrefutable_let_patterns)] // Text/Shape variants come later (plan.md §7)
    let ObjectKind::Image(data) = opened.session.document().find_object(id)?.kind() else {
        return None;
    };
    let data = data.clone();
    let current_page = opened.viewer.current_page() as usize;
    let page_count = opened.session.document().page_count();
    let kind_label = library
        .assets
        .iter()
        .find(|asset| asset.id() == data.asset_id)
        .map(|asset| asset.kind().label())
        .unwrap_or("Image");

    // "To page…" — one row per page, the current page disabled and checked
    // (same-page duplication is Duplicate's job). Row order equals page
    // order, so the menu index IS the page index.
    let to_page = (page_count > 1).then(|| {
        let weak = cx.weak_entity();
        let items: Vec<MenuItem> = (0..page_count)
            .map(|index| {
                MenuItem::new(format!("Page {}", index + 1))
                    .disabled(index == current_page)
                    .checked(index == current_page)
            })
            .collect();
        menu(
            "mark-toolbar-to-page",
            with_tooltip(
                button(
                    "mark-toolbar-to-page-trigger",
                    "To page…",
                    ButtonVariant::Secondary,
                    cx,
                ),
                "Place a copy on another page",
            ),
            items,
            move |index, _, cx| {
                weak.update(cx, |app, cx| app.duplicate_to_page(index, cx))
                    .ok();
            },
        )
        .into_any_element()
    });

    let duplicate_hint = if cfg!(target_os = "macos") {
        "Duplicate (⌘D)"
    } else {
        "Duplicate (Ctrl+D)"
    };

    Some(
        div()
            .id("mark-selection-toolbar")
            .test_support()
            .absolute()
            .top(rems(0.75))
            .right(rems(0.75))
            .flex()
            .items_center()
            .gap(rems(0.25))
            .px(rems(0.375))
            .py(rems(0.25))
            .border_1()
            .border_color(theme.border)
            .rounded_sm()
            .bg(theme.background)
            .shadow_md()
            // Keep presses inside the toolbar: a bubbling press would hit
            // the canvas viewport, clear the selection, and unmount the
            // toolbar mid-press (same trap as the resize handles).
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                |_: &gpui_kit::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                },
            )
            .child(
                div()
                    .id("mark-toolbar-kind")
                    .px(rems(0.25))
                    .text_size(rems(0.75))
                    .text_color(theme.secondary)
                    .child(kind_label),
            )
            .child(with_tooltip(
                icon_button(
                    "mark-toolbar-smaller",
                    IconName::Minus,
                    "Smaller",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.handle_resize_selected(0.9, cx)),
                ),
                "Smaller (−10%)",
            ))
            .child(with_tooltip(
                icon_button(
                    "mark-toolbar-bigger",
                    IconName::Plus,
                    "Bigger",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.handle_resize_selected(1.1, cx)),
                ),
                "Bigger (+10%)",
            ))
            .child(with_tooltip(
                button(
                    "mark-toolbar-duplicate",
                    "Duplicate",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_duplicate(cx))),
                duplicate_hint,
            ))
            .children(to_page)
            .child(with_tooltip(
                icon_button(
                    "mark-toolbar-delete",
                    IconName::Trash,
                    "Delete",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.handle_delete_selected(window, cx)
                })),
                "Delete",
            ))
            .into_any_element(),
    )
}

#[allow(clippy::too_many_arguments)]
fn header(
    theme: &Theme,
    title: Option<&str>,
    zoom_percent: Option<u32>,
    can_export: bool,
    exporting: bool,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    div()
        .id("mark-header")
        .flex()
        .items_center()
        .justify_between()
        .px(rems(1.))
        .h(rems(2.75))
        .bg(theme.background)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(rems(0.5))
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child("Mark"),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(title.unwrap_or("Sign anything.").to_owned()),
                ),
        )
        .child(
            div()
                .id("mark-header-actions")
                .flex()
                .items_center()
                .gap(rems(0.375))
                .children(zoom_percent.map(|percent| {
                    div()
                        .id("mark-zoom-controls")
                        .flex()
                        .items_center()
                        .gap(rems(0.25))
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-out",
                                IconName::ZoomOut,
                                "Zoom out",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_out(cx)),
                            ),
                            "Zoom out (-)",
                        ))
                        .child(
                            div()
                                .id("mark-zoom-percent")
                                .text_size(rems(0.75))
                                .text_color(theme.secondary)
                                .min_w(rems(2.5))
                                .flex()
                                .justify_center()
                                .child(format!("{percent}%")),
                        )
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-in",
                                IconName::ZoomIn,
                                "Zoom in",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_in(cx)),
                            ),
                            "Zoom in (+)",
                        ))
                        .child(gpui_omarchy::with_tooltip(
                            gpui_omarchy::icon_button(
                                "zoom-fit",
                                IconName::Maximize,
                                "Fit page",
                                ButtonVariant::Secondary,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.handle_zoom_fit(cx)),
                            ),
                            "Fit page (0)",
                        ))
                }))
                .children(export_button(can_export, exporting, cx)),
        )
}

/// The header's Export control (plan.md §10): primary action, hidden
/// without a document, disabled while an export runs.
fn export_button(
    can_export: bool,
    exporting: bool,
    cx: &mut Context<MarkApp>,
) -> Option<impl IntoElement> {
    can_export.then(|| {
        let export_hint = if cfg!(target_os = "macos") {
            "Export (⌘S)"
        } else {
            "Export (Ctrl+S)"
        };
        gpui_omarchy::with_tooltip(
            button("mark-export-button", "Export", ButtonVariant::Primary, cx)
                .disabled(exporting)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.handle_export(window, cx);
                })),
            export_hint,
        )
    })
}

/// Status bar: page navigation and indication on the left; export
/// feedback and zoom on the right (plan.md §10, §16).
#[allow(clippy::too_many_arguments)]
fn status_bar(
    theme: &Theme,
    current_page: u32,
    page_count: usize,
    has_previous: bool,
    has_next: bool,
    zoom_percent: Option<u32>,
    export_status: Option<&ExportStatus>,
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    div()
        .id("mark-status-bar")
        .flex()
        .items_center()
        .justify_between()
        .px(rems(1.))
        .h(rems(2.))
        .flex_shrink_0()
        .bg(theme.background)
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .id("mark-status-pages")
                .flex()
                .items_center()
                .gap(rems(0.25))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "first-page",
                        IconName::ChevronsLeft,
                        "First page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_previous)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.handle_first_page(cx)),
                    ),
                    "First page (Home)",
                ))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "previous-page",
                        IconName::ChevronLeft,
                        "Previous page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_previous)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.handle_previous_page(cx)),
                    ),
                    "Previous page (Page Up)",
                ))
                .child(
                    div()
                        .id("mark-status-page-indicator")
                        .px(rems(0.5))
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(format!("Page {} / {}", current_page + 1, page_count)),
                )
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "next-page",
                        IconName::ChevronRight,
                        "Next page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_next)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_next_page(cx))),
                    "Next page (Page Down)",
                ))
                .child(gpui_omarchy::with_tooltip(
                    gpui_omarchy::icon_button(
                        "last-page",
                        IconName::ChevronsRight,
                        "Last page",
                        ButtonVariant::Secondary,
                        cx,
                    )
                    .disabled(!has_next)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_last_page(cx))),
                    "Last page (End)",
                )),
        )
        .child(
            div()
                .id("mark-status-right")
                .flex()
                .items_center()
                .gap(rems(0.5))
                .children(export_status.map(|status| {
                    let text = match status {
                        ExportStatus::Running { done, total } => {
                            format!("Exporting… {done}/{total}")
                        }
                        ExportStatus::Done { name } => format!("Exported {name}"),
                        ExportStatus::Failed(message) => format!("Export failed. {message}"),
                    };
                    div()
                        .id("mark-export-status")
                        .test_support()
                        .text_size(rems(0.75))
                        .text_color(match status {
                            ExportStatus::Failed(_) => theme.danger,
                            _ => theme.secondary,
                        })
                        .child(text)
                }))
                .child(
                    div()
                        .id("mark-status-zoom")
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(zoom_percent.map(|p| format!("{p}%")).unwrap_or_default()),
                ),
        )
}

fn empty_workspace(
    theme: &Theme,
    recent: &[PathBuf],
    cx: &mut Context<MarkApp>,
) -> impl IntoElement {
    let recents_list = (!recent.is_empty()).then(|| {
        div()
            .id("mark-recent")
            .flex()
            .flex_col()
            .items_start()
            .gap(rems(0.25))
            .w(rems(22.))
            .child(
                div()
                    .text_size(rems(0.75))
                    .text_color(theme.secondary)
                    .child("Recent"),
            )
            .children(recent.iter().enumerate().map(|(index, path)| {
                let path = path.clone();
                let name: SharedString = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Document")
                    .to_owned()
                    .into();
                let directory: SharedString = path
                    .parent()
                    .and_then(|parent| parent.to_str())
                    .unwrap_or("")
                    .to_owned()
                    .into();
                div()
                    .id(("mark-recent-row", index))
                    .test_support()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(rems(0.125))
                    .py(rems(0.375))
                    .px(rems(0.5))
                    .rounded_sm()
                    .hover(|style| style.bg(theme.background))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.open_document(path.clone(), window, cx);
                        }),
                    )
                    .child(
                        div()
                            .text_size(rems(0.875))
                            .text_color(theme.foreground)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(rems(0.6875))
                            .text_color(theme.secondary)
                            .truncate()
                            .child(directory),
                    )
            }))
    });

    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(0.75))
                .child(
                    icon(IconName::FileText)
                        .size(rems(2.5))
                        .text_color(theme.secondary),
                )
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .text_color(theme.foreground)
                        .child("No document open"),
                )
                .child(
                    button("open-document", "Open…", ButtonVariant::Outline, cx).on_click(
                        cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.open_with_dialog(cx);
                        }),
                    ),
                )
                .child(
                    div()
                        .text_size(rems(0.75))
                        .text_color(theme.secondary)
                        .child(format!("or press {}", MarkApp::open_hint())),
                )
                .children(recents_list),
        )
}

fn notice_workspace(
    theme: &Theme,
    icon_name: IconName,
    title: String,
    detail: Option<&str>,
) -> impl IntoElement {
    let detail = detail.map(|detail| {
        div()
            .text_size(rems(0.75))
            .text_color(theme.secondary)
            .child(detail.to_owned())
    });

    div()
        .id("mark-workspace")
        .flex()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(0.75))
                .child(icon(icon_name).size(rems(2.5)).text_color(theme.secondary))
                .child(
                    div()
                        .text_size(rems(0.9375))
                        .text_color(theme.foreground)
                        .child(title),
                )
                .children(detail),
        )
}
