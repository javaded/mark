//! Recent documents persistence (plan.md §18).
//!
//! Layer D: a bounded, most-recent-first list of opened document paths in
//! a small JSON file inside the platform app-config directory. No
//! database, no metadata beyond the paths themselves.
//!
//! The list is derived data — trivially rebuilt by opening documents —
//! so unlike the asset library a damaged file degrades to an empty list
//! instead of surfacing an error: there is nothing user-created to lose.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many documents the list keeps.
const LIMIT: usize = 10;

/// Schema version; bump on breaking changes and migrate on open.
const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum RecentError {
    #[error("the recent documents file is damaged")]
    Damaged(#[from] serde_json::Error),
    #[error("the recent documents file could not be written")]
    Write(#[source] std::io::Error),
}

#[derive(Serialize, Deserialize)]
struct RecentFile {
    version: u32,
    paths: Vec<PathBuf>,
}

/// The most-recent-first list of documents the user opened.
#[derive(Clone, Debug, Default)]
pub struct RecentDocuments {
    paths: Vec<PathBuf>,
}

impl RecentDocuments {
    /// Loads the list from `path` (e.g. `<config-dir>/recent.json`).
    ///
    /// A missing file is an empty list. A file that exists but cannot be
    /// parsed is an error; the caller decides whether to degrade (the
    /// app starts with an empty list and overwrites it on the next
    /// open — recent files are convenience data, not user assets).
    pub fn open(path: &Path) -> Result<Self, RecentError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(RecentError::Write(error)),
        };
        let file: RecentFile = serde_json::from_slice(&bytes)?;
        Ok(Self {
            paths: if file.version == VERSION {
                file.paths
            } else {
                Vec::new()
            },
        })
    }

    /// The remembered document paths, most recent first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Records an opened document: moves it to the front, drops duplicates,
    /// and trims the list to its bound. Persistence is [`Self::save`]'s job,
    /// off the UI thread.
    pub fn record(&mut self, path: &Path) {
        self.paths.retain(|existing| existing != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(LIMIT);
    }

    /// Writes the list atomically (temp file + rename) so a crash never
    /// leaves a half-written file behind.
    pub fn save(&self, path: &Path) -> Result<(), RecentError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(RecentError::Write)?;
        }
        let file = RecentFile {
            version: VERSION,
            paths: self.paths.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(RecentError::Damaged)?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes).map_err(RecentError::Write)?;
        fs::rename(&temp, path).map_err(RecentError::Write)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_opens_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let recent =
            RecentDocuments::open(&dir.path().join("recent.json")).expect("missing file is empty");
        assert!(recent.paths().is_empty());
    }

    #[test]
    fn record_orders_dedupes_and_truncates() {
        let mut recent = RecentDocuments::default();
        let a = Path::new("/docs/a.pdf");
        let b = Path::new("/docs/b.png");
        recent.record(a);
        recent.record(b);
        recent.record(a); // re-open moves to front, no duplicate
        assert_eq!(recent.paths(), [a, b]);

        for index in 0..12 {
            recent.record(PathBuf::from(format!("/docs/n{index}.pdf")).as_path());
        }
        // The bound keeps the ten newest: n11 (front) down to n2 (back);
        // n0 and n1 fell off.
        assert_eq!(recent.paths().len(), LIMIT, "bounded list");
        assert_eq!(
            recent.paths().first(),
            Some(&PathBuf::from("/docs/n11.pdf"))
        );
        assert_eq!(recent.paths().last(), Some(&PathBuf::from("/docs/n2.pdf")));
    }

    #[test]
    fn save_and_reopen_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/recent.json");
        let mut recent = RecentDocuments::default();
        recent.record(Path::new("/docs/contract.pdf"));
        recent.save(&path).expect("save creates parent dirs");

        let reloaded = RecentDocuments::open(&path).expect("reload");
        assert_eq!(reloaded.paths(), [PathBuf::from("/docs/contract.pdf")]);
    }

    #[test]
    fn damaged_file_errors_and_can_be_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("recent.json");
        std::fs::write(&path, b"not json").expect("write damaged file");
        assert!(RecentDocuments::open(&path).is_err());

        // The app degrades to a fresh list and overwrites the damage.
        let mut fresh = RecentDocuments::default();
        fresh.record(Path::new("/docs/ok.pdf"));
        fresh.save(&path).expect("overwrite");
        assert_eq!(
            RecentDocuments::open(&path).expect("reload").paths(),
            [PathBuf::from("/docs/ok.pdf")]
        );
    }
}
