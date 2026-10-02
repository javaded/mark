//! Command-based undo (plan.md §14).
//!
//! One user action = one command. A drag that spans many pointer events
//! commits exactly one `MoveObject`. Commands record enough state to reverse
//! themselves; all mutations of [`Document`] objects flow through here.

use crate::ids::{ObjectId, PageId};
use crate::model::{Document, DocumentObject, Rect, Vec2};

pub trait Command {
    /// Apply the change to the document. May record information needed to
    /// reverse it (e.g. the removed object's index).
    fn apply(&mut self, doc: &mut Document);

    /// Reverse the change. Must exactly restore the prior document state.
    fn undo(&mut self, doc: &mut Document);

    /// Short human-readable description, e.g. for the undo menu.
    fn describe(&self) -> String;
}

pub struct AddObject {
    pub page_id: PageId,
    pub object: DocumentObject,
}

impl AddObject {
    pub fn new(page_id: PageId, object: DocumentObject) -> Self {
        Self { page_id, object }
    }
}

impl Command for AddObject {
    fn apply(&mut self, doc: &mut Document) {
        doc.push_object(self.page_id, self.object.clone());
    }

    fn undo(&mut self, doc: &mut Document) {
        doc.remove_object(self.object.id());
    }

    fn describe(&self) -> String {
        format!("Add {}", self.object.kind().label())
    }
}

pub struct DeleteObject {
    pub object_id: ObjectId,
    /// Recorded by `apply`: where the object lived, so undo restores order.
    removed: Option<(PageId, usize, DocumentObject)>,
}

impl DeleteObject {
    pub fn new(object_id: ObjectId) -> Self {
        Self {
            object_id,
            removed: None,
        }
    }
}

impl Command for DeleteObject {
    fn apply(&mut self, doc: &mut Document) {
        self.removed = doc.remove_object(self.object_id);
    }

    fn undo(&mut self, doc: &mut Document) {
        if let Some((page_id, index, object)) = self.removed.take() {
            doc.insert_object_at(page_id, index, object);
        }
    }

    fn describe(&self) -> String {
        "Delete object".into()
    }
}

pub struct MoveObject {
    pub object_id: ObjectId,
    pub from: Vec2,
    pub to: Vec2,
}

impl MoveObject {
    pub fn new(object_id: ObjectId, from: Vec2, to: Vec2) -> Self {
        Self {
            object_id,
            from,
            to,
        }
    }

    fn set_position(doc: &mut Document, id: ObjectId, position: Vec2) {
        if let Some(image) = doc
            .object_mut(id)
            .and_then(|object| object.image_data_mut())
        {
            image.x = position.x;
            image.y = position.y;
        }
    }
}

impl Command for MoveObject {
    fn apply(&mut self, doc: &mut Document) {
        Self::set_position(doc, self.object_id, self.to);
    }

    fn undo(&mut self, doc: &mut Document) {
        Self::set_position(doc, self.object_id, self.from);
    }

    fn describe(&self) -> String {
        "Move object".into()
    }
}

/// Resizing can move the top-left corner (dragging the opposite handle), so
/// the command carries the full rect before and after.
pub struct ResizeObject {
    pub object_id: ObjectId,
    pub from: Rect,
    pub to: Rect,
}

impl ResizeObject {
    pub fn new(object_id: ObjectId, from: Rect, to: Rect) -> Self {
        Self {
            object_id,
            from,
            to,
        }
    }

    fn set_rect(doc: &mut Document, id: ObjectId, rect: Rect) {
        if let Some(image) = doc
            .object_mut(id)
            .and_then(|object| object.image_data_mut())
        {
            image.x = rect.x;
            image.y = rect.y;
            image.width = rect.width;
            image.height = rect.height;
        }
    }
}

impl Command for ResizeObject {
    fn apply(&mut self, doc: &mut Document) {
        Self::set_rect(doc, self.object_id, self.to);
    }

    fn undo(&mut self, doc: &mut Document) {
        Self::set_rect(doc, self.object_id, self.from);
    }

    fn describe(&self) -> String {
        "Resize object".into()
    }
}

pub struct RotateObject {
    pub object_id: ObjectId,
    pub from_degrees: f32,
    pub to_degrees: f32,
}

impl RotateObject {
    pub fn new(object_id: ObjectId, from_degrees: f32, to_degrees: f32) -> Self {
        Self {
            object_id,
            from_degrees,
            to_degrees,
        }
    }

    fn set_rotation(doc: &mut Document, id: ObjectId, degrees: f32) {
        if let Some(image) = doc
            .object_mut(id)
            .and_then(|object| object.image_data_mut())
        {
            image.rotation = degrees;
        }
    }
}

impl Command for RotateObject {
    fn apply(&mut self, doc: &mut Document) {
        Self::set_rotation(doc, self.object_id, self.to_degrees);
    }

    fn undo(&mut self, doc: &mut Document) {
        Self::set_rotation(doc, self.object_id, self.from_degrees);
    }

    fn describe(&self) -> String {
        "Rotate object".into()
    }
}

pub struct SetOpacity {
    pub object_id: ObjectId,
    pub from: f32,
    pub to: f32,
}

impl SetOpacity {
    pub fn new(object_id: ObjectId, from: f32, to: f32) -> Self {
        Self {
            object_id,
            from,
            to,
        }
    }

    fn set_opacity(doc: &mut Document, id: ObjectId, opacity: f32) {
        if let Some(image) = doc
            .object_mut(id)
            .and_then(|object| object.image_data_mut())
        {
            image.opacity = opacity;
        }
    }
}

impl Command for SetOpacity {
    fn apply(&mut self, doc: &mut Document) {
        Self::set_opacity(doc, self.object_id, self.to);
    }

    fn undo(&mut self, doc: &mut Document) {
        Self::set_opacity(doc, self.object_id, self.from);
    }

    fn describe(&self) -> String {
        "Set opacity".into()
    }
}

/// Duplicates an object onto the same page. The duplicate is a full copy
/// with a fresh id (still referencing the same asset).
pub struct DuplicateObject {
    pub source_id: ObjectId,
    pub duplicate: DocumentObject,
    pub offset: Vec2,
}

impl DuplicateObject {
    pub fn new(source_id: ObjectId, duplicate: DocumentObject, offset: Vec2) -> Self {
        // The duplicate keeps the source's data but gets a fresh identity,
        // offset so it lands beside the original (plan.md §27).
        let (_source_clone_id, mut kind) = duplicate.into_parts();
        #[allow(irrefutable_let_patterns)] // Text/Shape variants come later (plan.md §7)
        if let crate::model::ObjectKind::Image(data) = &mut kind {
            data.x += offset.x;
            data.y += offset.y;
        }
        Self {
            source_id,
            duplicate: DocumentObject::with_id(ObjectId::new(), kind),
            offset,
        }
    }
}

impl Command for DuplicateObject {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(page_id) = doc.page_of(self.source_id) {
            doc.push_object(page_id, self.duplicate.clone());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        doc.remove_object(self.duplicate.id());
    }

    fn describe(&self) -> String {
        "Duplicate object".into()
    }
}

/// Places a copy of an object onto another page (plan.md §26).
pub struct DuplicateToPage {
    pub duplicate: DocumentObject,
    pub to_page: PageId,
}

impl DuplicateToPage {
    pub fn new(duplicate: DocumentObject, to_page: PageId) -> Self {
        Self { duplicate, to_page }
    }
}

impl Command for DuplicateToPage {
    fn apply(&mut self, doc: &mut Document) {
        doc.push_object(self.to_page, self.duplicate.clone());
    }

    fn undo(&mut self, doc: &mut Document) {
        doc.remove_object(self.duplicate.id());
    }

    fn describe(&self) -> String {
        "Duplicate to page".into()
    }
}

/// Moves an object from one page to another, preserving z-order where
/// practical.
pub struct MoveObjectToPage {
    pub object_id: ObjectId,
    pub to_page: PageId,
    /// Recorded by `apply`: original location for undo.
    origin: Option<(PageId, usize, DocumentObject)>,
}

impl MoveObjectToPage {
    pub fn new(object_id: ObjectId, to_page: PageId) -> Self {
        Self {
            object_id,
            to_page,
            origin: None,
        }
    }
}

impl Command for MoveObjectToPage {
    fn apply(&mut self, doc: &mut Document) {
        self.origin = doc.remove_object(self.object_id);
        if let Some((_, _, object)) = self.origin.as_ref() {
            doc.push_object(self.to_page, object.clone());
        }
    }

    fn undo(&mut self, doc: &mut Document) {
        doc.remove_object(self.object_id);
        if let Some((page_id, index, object)) = self.origin.take() {
            doc.insert_object_at(page_id, index, object);
        }
    }

    fn describe(&self) -> String {
        "Move to page".into()
    }
}
