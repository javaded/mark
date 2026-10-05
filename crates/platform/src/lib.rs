//! Platform integration for Mark.
//!
//! Native file dialogs (`rfd`), application data directories, and any
//! genuinely platform-specific code. `#[cfg(target_os = ...)]` lives here
//! and nowhere else (plan.md §19.2).
//!
//! Dialogs stay behind the [`FilePicker`] seam so automated tests can
//! substitute a fake picker; native dialogs never run in tests (plan.md
//! §20.5).

use std::path::{Path, PathBuf};

use rfd::AsyncFileDialog;

/// Mark's per-user application data directory (plan.md §12, §18).
///
/// Linux `~/.local/share/mark`, macOS `~/Library/Application Support/mark`,
/// Windows `%APPDATA%\mark`. `None` when the platform has no discoverable
/// data directory (unusual; callers degrade to a session-only library).
pub fn app_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("mark"))
}

/// Mark's per-user application config directory (plan.md §18: recent
/// documents and, later, settings).
///
/// Linux `~/.config/mark`, macOS `~/Library/Application Support/mark`,
/// Windows `%APPDATA%\mark`. `None` when the platform has no discoverable
/// config directory; callers degrade to keeping recents in memory only.
pub fn app_config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        // macOS folds config into Application Support, like the data dir.
        app_data_dir()
    } else {
        dirs::config_dir().map(|dir| dir.join("mark"))
    }
}

/// `std::fs::canonicalize` with the Windows verbatim prefix stripped.
///
/// Canonical paths are what gets persisted (the recents list): resolved
/// symlinks survive `cwd` changes and 8.3 short names (`RUNNER~1`) become
/// real ones. But on Windows `canonicalize` returns `\\?\`-prefixed
/// verbatim paths — ugly in UI text and surprising to other tools — so
/// the prefix comes off (`\\?\C:\…` → `C:\…`, `\\?\UNC\server\…` →
/// `\\server\…`). A missing file returns the path as given.
pub fn canonicalize(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(canonical) => strip_windows_verbatim(canonical),
        Err(_) => path.to_path_buf(),
    }
}

/// Removes the `\\?\` verbatim prefix from an absolute Windows path.
fn strip_windows_verbatim(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path; // non-UTF-16-ish components: keep the verbatim form
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

/// Reveals a file in the platform file manager: macOS selects it in Finder
/// (`open -R`), Windows opens the folder with it selected
/// (`explorer /select,`), Linux opens the containing folder (`xdg-open`).
///
/// Fire-and-forget: the process is spawned detached and the handle dropped.
/// Errors (missing helper, headless session) are the caller's to log.
pub fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let parent = path.parent().unwrap_or(path);
        let mut command = std::process::Command::new("xdg-open");
        command.arg(parent);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    let mut command = std::process::Command::new("true");
    // The helper never gets our pipes: a file manager holding stdout open
    // would keep parents (shells, test harnesses) waiting on EOF.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
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

    /// Asks the user where to save the exported document, starting from
    /// `default_name` in the source's directory (plan.md §16).
    ///
    /// Resolves to `None` when the dialog is cancelled.
    #[allow(async_fn_in_trait)]
    async fn pick_save_export(&self, default_name: PathBuf) -> Option<PathBuf>;
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

    async fn pick_save_export(&self, default_name: PathBuf) -> Option<PathBuf> {
        let filter = if default_name.extension().and_then(|e| e.to_str()) == Some("pdf") {
            vec!["pdf"]
        } else {
            vec!["png"]
        };
        let name = default_name.file_name()?.to_str()?.to_owned();
        let directory = default_name.parent().map(Path::to_path_buf);
        let mut dialog = AsyncFileDialog::new()
            .set_title("Export signed document")
            .add_filter(filter_name(&filter), &filter)
            .set_file_name(name);
        if let Some(directory) = directory
            && !directory.as_os_str().is_empty()
        {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .save_file()
            .await
            .map(|file| file.path().to_path_buf())
    }
}

/// Display name for a save-dialog filter list.
fn filter_name(extensions: &[&str]) -> String {
    match extensions {
        ["pdf"] => "PDF document".to_owned(),
        _ => "PNG image".to_owned(),
    }
}
