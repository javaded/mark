//! Domain model tests (plan.md §20.2): every command applies and reverses,
//! sessions track dirty state, and the document invariants hold.

use std::path::PathBuf;

use mark_core::{
    AddObject, Asset, AssetId, AssetKind, DeleteObject, Document, DocumentObject, DocumentSession,
    DocumentSource, DuplicateObject, DuplicateToPage, ImageObject, MoveObject, MoveObjectToPage,
    ObjectId, Page, PageId, Rect, ResizeObject, RotateObject, SetOpacity, Vec2,
};

fn test_document() -> (DocumentSession, PageId, PageId) {
    let page1 = Page::new(612.0, 792.0); // US Letter portrait
    let page2 = Page::new(792.0, 612.0); // landscape
    let id1 = page1.id();
    let id2 = page2.id();
    let document = Document::new(
        DocumentSource::Pdf {
            path: PathBuf::from("/tmp/test.pdf"),
        },
        vec![page1, page2],
    );
    (DocumentSession::new(document), id1, id2)
}

fn image_object(page: &Page, asset_id: AssetId) -> DocumentObject {
    DocumentObject::image(ImageObject::centered_on_page(page, asset_id, 4.0))
}

fn image_data(session: &DocumentSession, id: ObjectId) -> mark_core::ImageObject {
    match session.document().find_object(id).unwrap().kind() {
        mark_core::ObjectKind::Image(data) => data.clone(),
    }
}

#[test]
fn pages_keep_identity_and_metadata() {
    let (session, id1, id2) = test_document();
    let document = session.document();
    assert_eq!(document.page_count(), 2);
    assert_ne!(id1, id2);
    assert_eq!(document.page(id1).unwrap().width(), 612.0);
    assert_eq!(document.page(id2).unwrap().height(), 612.0);
    assert_eq!(
        document.page(id1).unwrap().rotation().degrees(),
        0,
        "fresh pages have no rotation"
    );
}

#[test]
fn page_rotation_parses_degrees() {
    use mark_core::PageRotation;
    assert_eq!(PageRotation::from_degrees(0), PageRotation::None);
    assert_eq!(PageRotation::from_degrees(90), PageRotation::Degrees90);
    assert_eq!(PageRotation::from_degrees(270), PageRotation::Degrees270);
    assert_eq!(PageRotation::from_degrees(360), PageRotation::None);
    assert_eq!(PageRotation::from_degrees(450), PageRotation::Degrees90);
}

#[test]
fn centered_placement_uses_quarter_page_width_and_preserves_aspect() {
    let page = Page::new(612.0, 792.0);
    let object = ImageObject::centered_on_page(&page, AssetId::new(), 4.0);
    assert!((object.width - 153.0).abs() < 1e-4); // 612 * 0.25
    assert!((object.height - 38.25).abs() < 1e-4); // 153 / 4
    assert!((object.x - (612.0 - 153.0) / 2.0).abs() < 1e-4);
    assert!((object.y - (792.0 - 38.25) / 2.0).abs() < 1e-4);
    assert_eq!(object.rotation, 0.0);
    assert_eq!(object.opacity, 1.0);
}

#[test]
fn add_delete_round_trip_restores_z_order() {
    let (mut session, page_id, _) = test_document();
    let a = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let b = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let id_a = a.id();
    let id_b = b.id();

    session.execute(Box::new(AddObject::new(page_id, a)));
    session.execute(Box::new(AddObject::new(page_id, b)));
    assert_eq!(session.document().page(page_id).unwrap().objects().len(), 2);
    assert!(session.is_dirty());

    session.execute(Box::new(DeleteObject::new(id_a)));
    let objects = session.document().page(page_id).unwrap().objects();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id(), id_b, "remaining object is B");

    session.undo(); // undo delete
    let objects = session.document().page(page_id).unwrap().objects();
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].id(), id_a, "A restored at its original index");
    assert_eq!(objects[1].id(), id_b);
}

#[test]
fn move_object_undo_redo() {
    let (mut session, page_id, _) = test_document();
    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let id = object.id();
    let from = Vec2 { x: 100.0, y: 100.0 };
    let to = Vec2 { x: 300.0, y: 42.0 };
    session.execute(Box::new(AddObject::new(page_id, object)));
    session.execute(Box::new(MoveObject::new(id, from, to)));

    let moved = image_data(&session, id);
    assert_eq!((moved.x, moved.y), (300.0, 42.0));

    assert!(session.undo());
    assert_eq!(
        image_data(&session, id).x,
        100.0,
        "undo restores the from position"
    );

    assert!(session.redo());
    assert_eq!(image_data(&session, id).x, 300.0);
}

#[test]
fn resize_object_round_trip_includes_position() {
    let (mut session, page_id, _) = test_document();
    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let id = object.id();
    session.execute(Box::new(AddObject::new(page_id, object)));

    let from = Rect {
        x: 100.0,
        y: 100.0,
        width: 150.0,
        height: 40.0,
    };
    // Dragging the top-left handle moves the box and changes size.
    let to = Rect {
        x: 80.0,
        y: 90.0,
        width: 170.0,
        height: 50.0,
    };
    session.execute(Box::new(ResizeObject::new(id, from, to)));
    let resized = image_data(&session, id);
    assert_eq!((resized.x, resized.width), (80.0, 170.0));

    session.undo();
    let restored = image_data(&session, id);
    assert_eq!(
        (restored.x, restored.y, restored.width, restored.height),
        (100.0, 100.0, 150.0, 40.0)
    );
}

#[test]
fn rotate_and_opacity_round_trip() {
    let (mut session, page_id, _) = test_document();
    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let id = object.id();
    session.execute(Box::new(AddObject::new(page_id, object)));

    session.execute(Box::new(RotateObject::new(id, 0.0, -12.5)));
    session.execute(Box::new(SetOpacity::new(id, 1.0, 0.6)));
    let data = image_data(&session, id);
    assert_eq!((data.rotation, data.opacity), (-12.5, 0.6));

    session.undo();
    assert_eq!(image_data(&session, id).opacity, 1.0);
    session.undo();
    assert_eq!(image_data(&session, id).rotation, 0.0);
}

#[test]
fn duplicate_gets_fresh_id_and_offset_on_same_page() {
    let (mut session, page_id, _) = test_document();
    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let source_id = object.id();
    let original = match object.kind() {
        mark_core::ObjectKind::Image(data) => data.clone(),
    };
    session.execute(Box::new(AddObject::new(page_id, object)));

    // Clone of the source, as the UI would produce.
    let clone = DocumentObject::image(original.clone());
    session.execute(Box::new(DuplicateObject::new(
        source_id,
        clone,
        Vec2 { x: 12.0, y: 12.0 },
    )));

    let objects = session.document().page(page_id).unwrap().objects();
    assert_eq!(objects.len(), 2);
    assert_ne!(objects[0].id(), objects[1].id(), "duplicate has fresh id");
    let dup = match objects[1].kind() {
        mark_core::ObjectKind::Image(data) => data.clone(),
    };
    assert_eq!((dup.x, dup.y), (original.x + 12.0, original.y + 12.0));

    session.undo();
    assert_eq!(session.document().page(page_id).unwrap().objects().len(), 1);
}

#[test]
fn duplicate_to_other_page_and_back() {
    let (mut session, page1, page2) = test_document();
    let object = image_object(session.document().page(page1).unwrap(), AssetId::new());
    let data = match object.kind() {
        mark_core::ObjectKind::Image(data) => data.clone(),
    };
    session.execute(Box::new(AddObject::new(page1, object)));

    session.execute(Box::new(DuplicateToPage::new(
        DocumentObject::image(data.clone()),
        page2,
    )));
    assert_eq!(
        session.document().page(page1).unwrap().objects().len(),
        1,
        "source page untouched"
    );
    assert_eq!(session.document().page(page2).unwrap().objects().len(), 1);

    session.undo();
    assert_eq!(session.document().page(page2).unwrap().objects().len(), 0);
}

#[test]
fn move_object_to_page_round_trip_restores_original_index() {
    let (mut session, page1, page2) = test_document();
    let a = image_object(session.document().page(page1).unwrap(), AssetId::new());
    let b = image_object(session.document().page(page1).unwrap(), AssetId::new());
    let id_a = a.id();
    let id_b = b.id();
    session.execute(Box::new(AddObject::new(page1, a)));
    session.execute(Box::new(AddObject::new(page1, b)));

    session.execute(Box::new(MoveObjectToPage::new(id_a, page2)));
    let page1_objects = session.document().page(page1).unwrap().objects();
    let page2_objects = session.document().page(page2).unwrap().objects();
    assert_eq!(page1_objects.len(), 1);
    assert_eq!(page1_objects[0].id(), id_b);
    assert_eq!(page2_objects.len(), 1);
    assert_eq!(page2_objects[0].id(), id_a);

    session.undo();
    let page1_objects = session.document().page(page1).unwrap().objects();
    assert_eq!(page1_objects.len(), 2);
    assert_eq!(
        (page1_objects[0].id(), page1_objects[1].id()),
        (id_a, id_b),
        "original z-order restored"
    );
}

#[test]
fn new_command_invalidates_redo_history() {
    let (mut session, page_id, _) = test_document();
    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    let id = object.id();
    session.execute(Box::new(AddObject::new(page_id, object)));

    session.execute(Box::new(MoveObject::new(
        id,
        Vec2 { x: 0.0, y: 0.0 },
        Vec2 { x: 10.0, y: 10.0 },
    )));
    session.undo();
    assert!(session.undo_manager().can_redo());
    assert_eq!(
        session.undo_manager().next_redo_description().as_deref(),
        Some("Move object")
    );

    session.execute(Box::new(RotateObject::new(id, 0.0, 90.0)));
    assert!(
        !session.undo_manager().can_redo(),
        "redo history cleared by the new command"
    );
}

#[test]
fn dirty_generation_tracks_save_and_undo_semantics() {
    let (mut session, page_id, _) = test_document();
    assert!(!session.is_dirty(), "fresh session is clean");

    let object = image_object(session.document().page(page_id).unwrap(), AssetId::new());
    session.execute(Box::new(AddObject::new(page_id, object)));
    assert!(session.is_dirty());

    session.mark_saved();
    assert!(!session.is_dirty(), "mark_saved clears dirty");

    session.undo();
    assert!(
        session.is_dirty(),
        "undoing past the save point makes it dirty again"
    );

    session.redo();
    assert!(
        !session.is_dirty(),
        "redo back to saved generation is clean"
    );
}

#[test]
fn undo_on_empty_session_is_a_no_op() {
    let (mut session, _, _) = test_document();
    assert!(!session.undo());
    assert!(!session.redo());
    assert!(!session.is_dirty());
}

#[test]
fn assets_carry_identity_and_kind() {
    let asset = Asset::new(
        "Javad signature",
        AssetKind::Signature,
        PathBuf::from("/data/assets/sig.png"),
        320,
        120,
    );
    assert_eq!(asset.kind().label(), "Signature");
    assert_eq!(asset.name(), "Javad signature");
    assert_eq!(asset.image_path(), Path::new("/data/assets/sig.png"));
    assert!((asset.aspect() - 320. / 120.).abs() < 1e-6);
    assert_ne!(
        asset.id(),
        Asset::new("x", AssetKind::Stamp, PathBuf::new(), 1, 1).id()
    );
}

use std::path::Path;
