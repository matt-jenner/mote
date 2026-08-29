use photo_catalog::{WallCursorKey, WallOrder};
use photo_domain::AssetId;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Cursor {
    direction: crate::SortDirection,
    scope: photo_domain::GalleryScope,
    selection_id: String,
    group_id: String,
    key: Key,
}
#[derive(Serialize, Deserialize)]
enum Key {
    Provisional {
        order: u64,
        id: String,
    },
    Captured {
        captured_at_utc: String,
        display_path: String,
        id: String,
    },
}

pub fn encode_cursor(
    direction: crate::SortDirection,
    scope: photo_domain::GalleryScope,
    selection: &crate::GallerySelection,
    key: &WallCursorKey,
) -> Result<String, crate::AppServiceError> {
    let key = match key {
        WallCursorKey::Provisional { order, id } => Key::Provisional {
            order: *order,
            id: id.as_uuid().hyphenated().to_string(),
        },
        WallCursorKey::Captured {
            captured_at_utc,
            display_path,
            id,
        } => Key::Captured {
            captured_at_utc: captured_at_utc.clone(),
            display_path: display_path.clone(),
            id: id.as_uuid().hyphenated().to_string(),
        },
    };
    let bytes = serde_json::to_vec(&Cursor {
        direction,
        scope,
        selection_id: selection.id().to_owned(),
        group_id: selection.group_id().as_uuid().hyphenated().to_string(),
        key,
    })?;
    Ok(encode(&bytes))
}

pub fn decode_cursor(
    value: &str,
    direction: crate::SortDirection,
    scope: photo_domain::GalleryScope,
    selection: &crate::GallerySelection,
    order: WallOrder,
) -> Result<WallCursorKey, crate::AppServiceError> {
    if value.is_empty() || value.len() > 2048 || value.len() % 4 == 1 {
        return Err(crate::AppServiceError::InvalidCursor);
    }
    let bytes = decode(value).ok_or(crate::AppServiceError::InvalidCursor)?;
    if bytes.len() > 1536 {
        return Err(crate::AppServiceError::InvalidCursor);
    }
    let cursor: Cursor =
        serde_json::from_slice(&bytes).map_err(|_| crate::AppServiceError::InvalidCursor)?;
    if cursor.direction != direction
        || cursor.scope != scope
        || cursor.selection_id != selection.id()
        || cursor.group_id != selection.group_id().as_uuid().hyphenated().to_string()
    {
        return Err(crate::AppServiceError::InvalidCursor);
    }
    match (order, cursor.key) {
        (WallOrder::Provisional, Key::Provisional { order, id }) => {
            Ok(WallCursorKey::Provisional {
                order,
                id: parse_id(&id)?,
            })
        }
        (
            WallOrder::CapturedAscending | WallOrder::CapturedDescending,
            Key::Captured {
                captured_at_utc,
                display_path,
                id,
            },
        ) => Ok(WallCursorKey::Captured {
            captured_at_utc,
            display_path,
            id: parse_id(&id)?,
        }),
        _ => Err(crate::AppServiceError::InvalidCursor),
    }
}
fn parse_id(id: &str) -> Result<AssetId, crate::AppServiceError> {
    uuid::Uuid::parse_str(id)
        .map(AssetId::from_uuid)
        .map_err(|_| crate::AppServiceError::InvalidCursor)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn encode(input: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let a = input[i] as u32;
        let b = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let c = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };
        let n = (a << 16) | (b << 8) | c;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if i + 1 < input.len() {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char)
        }
        if i + 2 < input.len() {
            out.push(ALPHABET[(n & 63) as usize] as char)
        }
        i += 3;
    }
    out
}
fn decode(input: &str) -> Option<Vec<u8>> {
    let mut vals = Vec::new();
    for c in input.bytes() {
        vals.push(ALPHABET.iter().position(|v| *v == c)? as u32)
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < vals.len() {
        let n = (vals[i] << 18)
            | ((vals.get(i + 1).copied().unwrap_or(0)) << 12)
            | ((vals.get(i + 2).copied().unwrap_or(0)) << 6)
            | (vals.get(i + 3).copied().unwrap_or(0));
        out.push((n >> 16) as u8);
        if i + 2 < vals.len() {
            out.push((n >> 8) as u8)
        }
        if i + 3 < vals.len() {
            out.push(n as u8)
        }
        i += 4;
    }
    Some(out)
}
