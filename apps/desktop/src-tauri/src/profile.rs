use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("invalid profile name")]
    InvalidName,
}

pub struct ProfileRoots {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl ProfileRoots {
    pub fn from_bases(
        data: &Path,
        cache: &Path,
        profile: Option<&str>,
    ) -> Result<Self, ProfileError> {
        let Some(profile) = profile else {
            return Ok(Self {
                data_dir: data.to_owned(),
                cache_dir: cache.to_owned(),
            });
        };
        if profile.is_empty()
            || profile.len() > 48
            || !profile
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ProfileError::InvalidName);
        }
        Ok(Self {
            data_dir: data.join("profiles").join(profile),
            cache_dir: cache.join("profiles").join(profile),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::ProfileRoots;

    #[test]
    fn named_profile_isolated_below_both_app_roots() {
        let roots = ProfileRoots::from_bases(
            Path::new("/app-data"),
            Path::new("/app-cache"),
            Some("checkpoint-1"),
        )
        .unwrap();
        assert_eq!(
            roots.data_dir,
            PathBuf::from("/app-data/profiles/checkpoint-1")
        );
        assert_eq!(
            roots.cache_dir,
            PathBuf::from("/app-cache/profiles/checkpoint-1")
        );
    }

    #[test]
    fn profile_names_reject_separators_and_empty_values() {
        for value in ["", "../escape", "a/b", "a b", "."] {
            assert!(
                ProfileRoots::from_bases(Path::new("data"), Path::new("cache"), Some(value))
                    .is_err()
            );
        }
    }
}
