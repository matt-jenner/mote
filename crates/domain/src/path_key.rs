use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NativePathKey(Vec<u8>);

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RelativePathKey(NativePathKey);

#[derive(Debug, Error)]
pub enum PathKeyError {
    #[error("path must be relative")]
    Absolute,
    #[error("stored path has invalid native encoding")]
    InvalidEncoding,
}

impl RelativePathKey {
    pub fn from_relative_path(path: &Path) -> Result<Self, PathKeyError> {
        if path.is_absolute() {
            return Err(PathKeyError::Absolute);
        }

        Ok(Self(NativePathKey::from_path(path)))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PathKeyError> {
        let native = NativePathKey::from_bytes(bytes)?;
        if native.to_path_buf()?.is_absolute() {
            return Err(PathKeyError::Absolute);
        }

        Ok(Self(native))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn to_path_buf(&self) -> Result<PathBuf, PathKeyError> {
        self.0.to_path_buf()
    }
}

impl NativePathKey {
    pub fn from_path(path: &Path) -> Self {
        Self(encode_native(path))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PathKeyError> {
        let value = Self(bytes);
        value.to_path_buf()?;
        Ok(value)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_path_buf(&self) -> Result<PathBuf, PathKeyError> {
        decode_native(&self.0)
    }
}

#[cfg(unix)]
fn encode_native(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = vec![b'U'];
    encoded.extend_from_slice(path.as_os_str().as_bytes());
    encoded
}

#[cfg(unix)]
fn decode_native(bytes: &[u8]) -> Result<PathBuf, PathKeyError> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let payload = bytes
        .strip_prefix(b"U")
        .ok_or(PathKeyError::InvalidEncoding)?;
    Ok(PathBuf::from(OsStr::from_bytes(payload)))
}

#[cfg(windows)]
fn encode_native(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    let mut encoded = vec![b'W'];
    for unit in path.as_os_str().encode_wide() {
        encoded.extend_from_slice(&unit.to_le_bytes());
    }
    encoded
}

#[cfg(windows)]
fn decode_native(bytes: &[u8]) -> Result<PathBuf, PathKeyError> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let payload = bytes
        .strip_prefix(b"W")
        .ok_or(PathKeyError::InvalidEncoding)?;
    if payload.len() % 2 != 0 {
        return Err(PathKeyError::InvalidEncoding);
    }

    let wide = payload
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}
