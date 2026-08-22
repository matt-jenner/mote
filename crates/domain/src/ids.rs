use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RelativePathKey;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

id_type!(LibraryId);
id_type!(FolderGroupId);
id_type!(DerivativeId);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AssetId(Uuid);

impl AssetId {
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn for_path(library: LibraryId, path: &RelativePathKey) -> Self {
        Self(Uuid::new_v5(&library.as_uuid(), path.as_bytes()))
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}
