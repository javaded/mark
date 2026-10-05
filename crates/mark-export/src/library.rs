//! Signature/stamp asset library persistence (plan.md §12, §18).
//!
//! Layer D: the library lives in the application data directory as
//! normalized PNGs plus a JSON manifest. Imported images are decoded and
//! re-encoded once at import (JPEG/WebP cannot carry alpha and PDFium
//! cannot embed WebP, so every stored asset is RGBA PNG — the library is
//! self-contained and the user may delete the original file).
//!
//! The original asset file is never modified. Assets are added by
//! [`AssetLibrary::import`] and removed by [`AssetLibrary::remove`]
//! (manifest entry plus stored PNG — the user's original import source
//! is never touched).

use std::fs;
use std::path::{Path, PathBuf};

use mark_core::{Asset, AssetId, AssetKind};
use serde::{Deserialize, Serialize};

/// On-disk manifest: `<data-dir>/library.json`.
const MANIFEST_FILE: &str = "library.json";
/// Normalized PNGs: `<data-dir>/assets/<asset-id>.png`.
const ASSETS_DIR: &str = "assets";

/// Manifest schema version; bump on breaking changes and migrate on open.
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("the asset library directory could not be created")]
    CreateDir(#[source] std::io::Error),
    #[error("the asset could not be decoded")]
    Decode(#[source] mark_image::LoadImageError),
    #[error("the asset could not be encoded")]
    Encode(#[source] image::ImageError),
    #[error("the asset could not be written")]
    Write(#[source] std::io::Error),
    #[error("the library manifest is damaged")]
    Manifest(#[source] serde_json::Error),
}

/// The user's persistent signature/stamp library.
pub struct AssetLibrary {
    /// Application data directory owning `library.json` and `assets/`.
    root: PathBuf,
    assets: Vec<Asset>,
}

impl AssetLibrary {
    /// Opens the library stored under `root` (the app data directory).
    ///
    /// A missing directory or manifest is a fresh, empty library — the very
    /// first launch must succeed. A manifest that exists but cannot be
    /// parsed is an error: silently replacing it would hide the user's
    /// assets behind an empty library.
    pub fn open(root: &Path) -> Result<Self, LibraryError> {
        let assets = match fs::read(root.join(MANIFEST_FILE)) {
            Ok(bytes) => Some(Self::parse_manifest(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(LibraryError::Write(error)),
        };
        Ok(Self {
            root: root.to_path_buf(),
            assets: assets.unwrap_or_default(),
        })
    }

    /// All assets, in import order.
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// The application data directory this library persists in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.id() == id)
    }

    /// Imports `source` as a new asset of `kind`: decode, normalize to
    /// RGBA PNG inside the library, record it in the manifest.
    ///
    /// The asset name comes from the source file stem. Returns the stored
    /// asset; the caller re-renders from [`Self::load_image`].
    pub fn import(&mut self, source: &Path, kind: AssetKind) -> Result<Asset, LibraryError> {
        let (_, rgba) = mark_image::ImageDocument::load(source)
            .map_err(LibraryError::Decode)?
            .into_parts();
        let (width, height) = rgba.dimensions();

        let name = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("asset")
            .to_owned();
        // The asset id doubles as the PNG filename: unique, opaque, stable.
        let id = AssetId::new();
        let asset = Asset::from_parts(
            id,
            name,
            kind,
            PathBuf::from(format!("{ASSETS_DIR}/{id}.png")),
            width,
            height,
        );

        fs::create_dir_all(self.root.join(ASSETS_DIR)).map_err(LibraryError::CreateDir)?;
        rgba.save(self.root.join(asset.image_path()))
            .map_err(LibraryError::Encode)?;
        self.assets.push(asset.clone());
        self.save()?;
        Ok(asset)
    }

    /// Removes an asset from the library: the manifest loses its entry and
    /// its normalized PNG is deleted from the stored assets.
    ///
    /// Returns the removed asset, or `None` when the id was unknown
    /// (already removed — idempotent). The PNG deletion is best-effort:
    /// once the manifest is saved the file is unreferenced, and a file
    /// that is already gone is not an error.
    pub fn remove(&mut self, id: AssetId) -> Result<Option<Asset>, LibraryError> {
        let Some(index) = self.assets.iter().position(|asset| asset.id() == id) else {
            return Ok(None);
        };
        let asset = self.assets.remove(index);
        self.save()?;
        let _ = fs::remove_file(self.root.join(asset.image_path()));
        Ok(Some(asset))
    }

    /// Decodes an asset's normalized PNG for rendering.
    pub fn load_image(&self, id: AssetId) -> Result<image::RgbaImage, LibraryError> {
        load_asset_image(&self.root, id)
    }

    /// Writes the manifest atomically (temp file + rename) so a crash never
    /// leaves a half-written `library.json`.
    fn save(&self) -> Result<(), LibraryError> {
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            assets: self
                .assets
                .iter()
                .map(|asset| AssetEntry {
                    id: asset.id().to_string(),
                    name: asset.name().to_owned(),
                    kind: KindDto::from(asset.kind()),
                    file: asset
                        .image_path()
                        .to_str()
                        .expect("library-relative paths are valid UTF-8")
                        .to_owned(),
                    width: asset.pixel_width(),
                    height: asset.pixel_height(),
                })
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(LibraryError::Manifest)?;
        let path = self.root.join(MANIFEST_FILE);
        let tmp = self.root.join(format!("{MANIFEST_FILE}.tmp"));
        fs::write(&tmp, &bytes).map_err(LibraryError::Write)?;
        fs::rename(&tmp, &path).map_err(LibraryError::Write)?;
        Ok(())
    }

    fn parse_manifest(bytes: &[u8]) -> Result<Vec<Asset>, LibraryError> {
        let manifest: Manifest = serde_json::from_slice(bytes).map_err(LibraryError::Manifest)?;
        Ok(manifest
            .assets
            .into_iter()
            .filter_map(|entry| {
                let id = AssetId::parse(&entry.id)?;
                Some(Asset::from_parts(
                    id,
                    entry.name,
                    entry.kind.into(),
                    PathBuf::from(entry.file),
                    entry.width,
                    entry.height,
                ))
            })
            .collect())
    }
}

/// Decodes an asset's PNG without holding a library open — the background
/// loading path for UI caches.
pub fn load_asset_image(root: &Path, id: AssetId) -> Result<image::RgbaImage, LibraryError> {
    let library = AssetLibrary::open(root)?;
    let asset = library.asset(id).ok_or_else(|| {
        LibraryError::Write(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "asset not in library",
        ))
    })?;
    mark_image::ImageDocument::load(&library.root.join(asset.image_path()))
        .map(|document| document.into_parts().1)
        .map_err(LibraryError::Decode)
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    assets: Vec<AssetEntry>,
}

#[derive(Serialize, Deserialize)]
struct AssetEntry {
    id: String,
    name: String,
    kind: KindDto,
    file: String,
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize)]
enum KindDto {
    Signature,
    Stamp,
    Initials,
}

impl From<AssetKind> for KindDto {
    fn from(kind: AssetKind) -> Self {
        match kind {
            AssetKind::Signature => KindDto::Signature,
            AssetKind::Stamp => KindDto::Stamp,
            AssetKind::Initials => KindDto::Initials,
        }
    }
}

impl From<KindDto> for AssetKind {
    fn from(kind: KindDto) -> Self {
        match kind {
            KindDto::Signature => AssetKind::Signature,
            KindDto::Stamp => AssetKind::Stamp,
            KindDto::Initials => AssetKind::Initials,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn source_image(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mark-library-src-{name}-{}.png",
            std::process::id()
        ));
        RgbaImage::from_pixel(8, 4, Rgba([10, 200, 120, 128]))
            .save(&path)
            .expect("fixture write");
        path
    }

    #[test]
    fn open_on_missing_dir_is_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let library = AssetLibrary::open(&dir.path().join("nonexistent")).unwrap();
        assert!(library.assets().is_empty());
    }

    #[test]
    fn import_normalizes_to_png_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let source = source_image("import");

        let mut library = AssetLibrary::open(dir.path()).unwrap();
        let asset = library.import(&source, AssetKind::Signature).unwrap();

        assert_eq!(
            asset.name(),
            source.file_stem().and_then(|s| s.to_str()).unwrap()
        );
        assert_eq!((asset.pixel_width(), asset.pixel_height()), (8, 4));
        assert!((asset.aspect() - 2.0).abs() < 1e-6);

        // Stored as a real PNG with alpha preserved.
        let stored = dir.path().join(asset.image_path());
        let magic = fs::read(&stored).unwrap();
        assert_eq!(
            &magic[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(
            library.load_image(asset.id()).unwrap().get_pixel(0, 0),
            &Rgba([10, 200, 120, 128])
        );

        // The manifest round-trips identity, kind, and geometry.
        let reopened = AssetLibrary::open(dir.path()).unwrap();
        let persisted = reopened.asset(asset.id()).expect("asset persisted");
        assert_eq!(persisted.name(), asset.name());
        assert_eq!(persisted.kind(), AssetKind::Signature);
        assert_eq!((persisted.pixel_width(), persisted.pixel_height()), (8, 4));
        assert!(reopened.load_image(asset.id()).is_ok());
    }

    #[test]
    fn import_jpeg_source_becomes_png() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("scan.jpg");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            6,
            6,
            image::Rgb([90, 90, 240]),
        ))
        .save(&source)
        .unwrap();

        let mut library = AssetLibrary::open(dir.path()).unwrap();
        let asset = library.import(&source, AssetKind::Stamp).unwrap();
        assert_eq!(asset.kind(), AssetKind::Stamp);
        let magic = fs::read(dir.path().join(asset.image_path())).unwrap();
        assert_eq!(&magic[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn imports_keep_both_assets() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = AssetLibrary::open(dir.path()).unwrap();
        let first = library
            .import(&source_image("a"), AssetKind::Signature)
            .unwrap();
        let second = library
            .import(&source_image("b"), AssetKind::Signature)
            .unwrap();
        assert_ne!(first.id(), second.id());

        let reopened = AssetLibrary::open(dir.path()).unwrap();
        assert_eq!(reopened.assets().len(), 2);
    }

    #[test]
    fn damaged_manifest_is_an_error_not_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST_FILE), "{ not json").unwrap();
        assert!(AssetLibrary::open(dir.path()).is_err());
    }

    #[test]
    fn import_missing_file_fails_without_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = AssetLibrary::open(dir.path()).unwrap();
        assert!(
            library
                .import(Path::new("/nonexistent/mark-sig.png"), AssetKind::Signature)
                .is_err()
        );
        assert!(library.assets().is_empty());
        assert!(!dir.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn remove_drops_the_manifest_entry_and_the_stored_png() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = AssetLibrary::open(dir.path()).unwrap();
        let kept = library
            .import(&source_image("kept"), AssetKind::Signature)
            .unwrap();
        let removed = library
            .import(&source_image("gone"), AssetKind::Stamp)
            .unwrap();

        let removed = library.remove(removed.id()).unwrap().expect("removed");
        assert_eq!(removed.kind(), AssetKind::Stamp);
        assert!(!dir.path().join(removed.image_path()).exists());

        // The survivor is intact, on disk and in the manifest.
        let reopened = AssetLibrary::open(dir.path()).unwrap();
        assert_eq!(reopened.assets().len(), 1);
        assert!(reopened.asset(kept.id()).is_some());
        assert!(dir.path().join(kept.image_path()).exists());
        assert!(reopened.load_image(kept.id()).is_ok());
    }

    #[test]
    fn remove_unknown_id_is_none_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = AssetLibrary::open(dir.path()).unwrap();
        let asset = library
            .import(&source_image("idem"), AssetKind::Signature)
            .unwrap();
        let before = fs::read(dir.path().join(MANIFEST_FILE)).unwrap();

        assert!(library.remove(AssetId::new()).unwrap().is_none());
        assert!(library.remove(AssetId::new()).unwrap().is_none());

        // Nothing changed: same manifest bytes, same assets, same file.
        assert_eq!(fs::read(dir.path().join(MANIFEST_FILE)).unwrap(), before);
        assert!(library.asset(asset.id()).is_some());
    }
}
