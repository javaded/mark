//! Platform integration for Mark.
//!
//! Native file dialogs (`rfd`), application data directories, and any
//! genuinely platform-specific code. `#[cfg(target_os = ...)]` lives here
//! and nowhere else (plan.md §19.2).
//!
//! Dialogs stay behind the [`FilePicker`] seam so automated tests can
//! substitute a fake picker; native dialogs never run in tests (plan.md
//! §20.5).

use std::path::PathBuf;

use rfd::AsyncFileDialog;

/// Mark's per-user application data directory (plan.md §12, §18).
///
/// Linux `~/.local/share/mark`, macOS `~/Library/Application Support/mark`,
/// Windows `%APPDATA%\mark`. `None` when the platform has no discoverable
/// data directory (unusual; callers degrade to a session-only library).
pub fn app_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("mark"))
}

/// Source of native open/save dialogs (plan.md §9.1).
pub trait FilePicker {
    /// Asks the user to pick one existing document.
    ///
    /// Resolves to `None` when the dialog is cancelled.
    #[allow(async_fn_in_trait)] // internal seam, never `dyn`, no auto-trait bounds needed
    async fn pick_open_document(&self) -> Option<PathBuf>;

    /// Asks the user to pick an image to import as a signature/stamp
    /// (plan.md §9.3: png, jpg/jpeg, webp).
    ///
    /// Resolves to `None` when the dialog is cancelled.
    #[allow(async_fn_in_trait)]
    async fn pick_open_image(&self) -> Option<PathBuf>;
}

/// Native file dialogs via `rfd`.
///
/// On Wayland this goes through xdg-desktop-portal (rfd default features:
/// `xdg-portal` + `wayland`); no GTK is required. The dialog future must be
/// awaited on an executor (GPUI's foreground executor polls it fine — the
/// portal backend is runtime-agnostic; rfd's own sync API blocks on the
/// same future with `pollster`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeFileDialogs;

impl FilePicker for NativeFileDialogs {
    async fn pick_open_document(&self) -> Option<PathBuf> {
        AsyncFileDialog::new()
            .set_title("Open document")
            .add_filter("Documents", &["pdf", "png", "jpg", "jpeg", "webp"])
            .pick_file()
            .await
            .map(|file| file.path().to_path_buf())
    }

    async fn pick_open_image(&self) -> Option<PathBuf> {
        AsyncFileDialog::new()
            .set_title("Import image")
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
            .await
            .map(|file| file.path().to_path_buf())
    }
}
