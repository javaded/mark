//! Core document model: sources, pages, placed objects, and reusable assets.

use std::path::{Path, PathBuf};

use crate::ids::{AssetId, ObjectId, PageId};

/// Where a document came from. The model does not care how a page is
/// rendered — engines (Layer C) interpret the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentSource {
    Image { path: PathBuf },
    Pdf { path: PathBuf },
}

impl DocumentSource {
    pub fn path(&self) -> &Path {
        match self {
            DocumentSource::Image { path } | DocumentSource::Pdf { path } => path,
        }
    }
}

/// A single page: logical dimensions plus the objects placed on it.
///
/// `width`/`height` are in logical page units (1 inch = 72 points), already
/// accounting for the page's intrinsic rotation as displayed.
#[derive(Debug, Clone)]
pub struct Page {
    id: PageId,
    width: f32,
    height: f32,
    rotation: PageRotation,
    objects: Vec<DocumentObject>,
}

impl Page {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            id: PageId::new(),
            width,
            height,
            rotation: PageRotation::None,
            objects: Vec::new(),
        }
    }

    pub fn id(&self) -> PageId {
        self.id
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn rotation(&self) -> PageRotation {
        self.rotation
    }

    pub fn objects(&self) -> &[DocumentObject] {
        &self.objects
    }

    pub(crate) fn object_index(&self, id: ObjectId) -> Option<usize> {
        self.objects.iter().position(|object| object.id() == id)
    }
}

/// Intrinsic PDF page rotation, displayed by the viewer (plan.md §34).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageRotation {
    #[default]
    None,
    Degrees90,
    Degrees180,
    Degrees270,
}

impl PageRotation {
    pub fn degrees(&self) -> u16 {
        match self {
            PageRotation::None => 0,
            PageRotation::Degrees90 => 90,
            PageRotation::Degrees180 => 180,
            PageRotation::Degrees270 => 270,
        }
    }

    pub fn from_degrees(degrees: u16) -> Self {
        match degrees % 360 {
            90 => PageRotation::Degrees90,
            180 => PageRotation::Degrees180,
            270 => PageRotation::Degrees270,
            _ => PageRotation::None,
        }
    }
}

/// An object placed on a page. The id is uniform across kinds so commands
/// and selection can reference any object type.
#[derive(Debug, Clone)]
pub struct DocumentObject {
    id: ObjectId,
    kind: ObjectKind,
}

impl DocumentObject {
    /// Creates an image object referencing a reusable asset.
    pub fn image(data: ImageObject) -> Self {
        Self {
            id: ObjectId::new(),
            kind: ObjectKind::Image(data),
        }
    }

    pub(crate) fn with_id(id: ObjectId, kind: ObjectKind) -> Self {
        Self { id, kind }
    }

    pub fn id(&self) -> ObjectId {
        self.id
    }

    pub fn kind(&self) -> &ObjectKind {
        &self.kind
    }

    pub(crate) fn into_parts(self) -> (ObjectId, ObjectKind) {
        (self.id, self.kind)
    }

    pub(crate) fn image_data_mut(&mut self) -> Option<&mut ImageObject> {
        match &mut self.kind {
            ObjectKind::Image(data) => Some(data),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ObjectKind {
    Image(ImageObject),
    // Future: Text, Shape (plan.md §7).
}

impl ObjectKind {
    /// Human-readable kind name for undo descriptions and accessibility.
    pub fn label(&self) -> &'static str {
        match self {
            ObjectKind::Image(_) => "image",
        }
    }
}

/// Placement of an image (signature/stamp) on a page.
///
/// `x`/`y` locate the top-left corner of the (unrotated) bounding box in
/// page coordinates; `rotation` is clockwise in degrees around the box
/// center; `opacity` is 0.0..=1.0.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageObject {
    pub asset_id: AssetId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: f32,
    pub opacity: f32,
}

impl ImageObject {
    pub fn centered_on_page(page: &Page, asset_id: AssetId, aspect: f32) -> Self {
        // Initial size ≈ 25% of page width, aspect preserved (plan.md §36).
        let width = page.width() * 0.25;
        let height = width / aspect;
        Self {
            asset_id,
            x: (page.width() - width) / 2.0,
            y: (page.height() - height) / 2.0,
            width,
            height,
            rotation: 0.0,
            opacity: 1.0,
        }
    }
}

/// A reusable signature/stamp asset in the user's library (plan.md §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    id: AssetId,
    name: String,
    kind: AssetKind,
    image_path: PathBuf,
}

impl Asset {
    pub fn new(name: impl Into<String>, kind: AssetKind, image_path: PathBuf) -> Self {
        Self {
            id: AssetId::new(),
            name: name.into(),
            kind,
            image_path,
        }
    }

    pub fn id(&self) -> AssetId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    pub fn image_path(&self) -> &Path {
        &self.image_path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Signature,
    Stamp,
    Initials,
}

impl AssetKind {
    pub fn label(&self) -> &'static str {
        match self {
            AssetKind::Signature => "Signature",
            AssetKind::Stamp => "Stamp",
            AssetKind::Initials => "Initials",
        }
    }
}

/// Axis-aligned rectangle in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Two-component vector in page or screen space, depending on context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// The whole document: its source plus its pages.
///
/// Dirty tracking lives in [`crate::DocumentSession`]; this type is pure
/// content so it can be snapshotted and exported.
#[derive(Debug, Clone)]
pub struct Document {
    source: DocumentSource,
    pages: Vec<Page>,
}

impl Document {
    pub fn new(source: DocumentSource, pages: Vec<Page>) -> Self {
        Self { source, pages }
    }

    pub fn source(&self) -> &DocumentSource {
        &self.source
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.pages.iter().find(|page| page.id() == id)
    }

    pub(crate) fn page_mut(&mut self, id: PageId) -> Option<&mut Page> {
        self.pages.iter_mut().find(|page| page.id() == id)
    }

    pub fn find_object(&self, id: ObjectId) -> Option<&DocumentObject> {
        self.pages
            .iter()
            .find_map(|page| page.objects().iter().find(|object| object.id() == id))
    }

    /// Page containing the given object, if any.
    pub fn page_of(&self, id: ObjectId) -> Option<PageId> {
        self.pages
            .iter()
            .find(|page| page.object_index(id).is_some())
            .map(Page::id)
    }

    // Object mutation is crate-private: all changes go through commands
    // (plan.md §14) so undo/redo stays coherent.

    pub(crate) fn push_object(&mut self, page_id: PageId, object: DocumentObject) -> Option<()> {
        self.page_mut(page_id)?.objects.push(object);
        Some(())
    }

    pub(crate) fn insert_object_at(
        &mut self,
        page_id: PageId,
        index: usize,
        object: DocumentObject,
    ) -> Option<()> {
        let page = self.page_mut(page_id)?;
        let index = index.min(page.objects.len());
        page.objects.insert(index, object);
        Some(())
    }

    pub(crate) fn remove_object(
        &mut self,
        id: ObjectId,
    ) -> Option<(PageId, usize, DocumentObject)> {
        let page_index = self
            .pages
            .iter()
            .position(|page| page.object_index(id).is_some())?;
        let page = &mut self.pages[page_index];
        let index = page.object_index(id)?;
        let object = page.objects.remove(index);
        Some((page.id(), index, object))
    }

    pub(crate) fn object_mut(&mut self, id: ObjectId) -> Option<&mut DocumentObject> {
        self.pages
            .iter_mut()
            .find_map(|page| page.objects.iter_mut().find(|object| object.id() == id))
    }
}
