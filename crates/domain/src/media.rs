use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LibraryKind {
    Configured,
    Recent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Availability {
    Available,
    RootOffline,
    Missing,
    Unreadable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MediaKind {
    Jpeg,
    Png,
    Tiff,
    Heif,
    Webp,
    Avif,
    Raw,
    Video,
    Unknown,
}

impl MediaKind {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "heic" | "heif" => Some(Self::Heif),
            "webp" => Some(Self::Webp),
            "avif" => Some(Self::Avif),
            "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" | "pef" => {
                Some(Self::Raw)
            }
            "mp4" | "mov" | "m4v" | "avi" | "mkv" => Some(Self::Video),
            _ => None,
        }
    }

    pub const fn is_wall_viewable(self) -> bool {
        match self {
            Self::Jpeg | Self::Png | Self::Tiff | Self::Webp => true,
            Self::Heif => cfg!(feature = "heic"),
            Self::Avif | Self::Raw | Self::Video | Self::Unknown => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileSignature {
    pub size_bytes: u64,
    pub modified_unix_ns: i128,
    pub sidecar_modified_unix_ns: Option<i128>,
}
