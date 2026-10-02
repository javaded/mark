//! Stable identifiers for model entities.
//!
//! UUIDs so identifiers stay unique across sessions — asset ids persist in
//! the signature library (plan.md §12) and must not collide after restarts.

use uuid::Uuid;

macro_rules! entity_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

entity_id!(
    /// Unique identifier of a page within a document.
    PageId
);
entity_id!(
    /// Unique identifier of an object placed on a page.
    ObjectId
);
entity_id!(
    /// Unique identifier of a reusable signature/stamp asset.
    AssetId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct() {
        assert_ne!(ObjectId::new(), ObjectId::new());
        assert_ne!(PageId::new(), PageId::new());
        assert_ne!(AssetId::new(), AssetId::new());
    }
}
