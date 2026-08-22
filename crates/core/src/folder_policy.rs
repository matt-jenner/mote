use std::path::{Path, PathBuf};

use globset::{Glob, GlobMatcher, GlobSet, GlobSetBuilder};
use photo_domain::{FolderPolicy, PathRatingRule};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderStructureSnapshot {
    children: Vec<PathBuf>,
}

impl FolderStructureSnapshot {
    pub fn new<I, P>(children: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        Self {
            children: children
                .into_iter()
                .map(|child| child.as_ref().to_path_buf())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    pub group_path: PathBuf,
    pub navigation_path: PathBuf,
    pub visible_by_default: bool,
    pub path_rating: Option<u8>,
    pub matched_policy: Option<String>,
}

#[derive(Debug, Error)]
pub enum FolderPolicyError {
    #[error("invalid folder-policy glob: {0}")]
    InvalidGlob(#[from] globset::Error),
    #[error("asset path must be relative")]
    AbsolutePath,
}

pub struct FolderPolicyEngine {
    policies: Vec<CompiledPolicy>,
}

struct CompiledPolicy {
    policy: FolderPolicy,
    required_children: Vec<GlobMatcher>,
    default_includes: GlobSet,
    navigation_hides: GlobSet,
}

impl FolderPolicyEngine {
    pub fn new(policies: Vec<FolderPolicy>) -> Result<Self, FolderPolicyError> {
        let policies = policies
            .into_iter()
            .map(CompiledPolicy::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { policies })
    }

    pub fn classify(
        &self,
        asset_path: &Path,
        structure: &FolderStructureSnapshot,
    ) -> Result<PolicyDecision, FolderPolicyError> {
        if asset_path.is_absolute() {
            return Err(FolderPolicyError::AbsolutePath);
        }

        for compiled in &self.policies {
            if compiled.matches_structure(structure) {
                return Ok(compiled.classify(asset_path));
            }
        }

        let navigation_path = asset_path.parent().unwrap_or(Path::new("")).to_path_buf();
        Ok(PolicyDecision {
            group_path: navigation_path.clone(),
            navigation_path,
            visible_by_default: true,
            path_rating: None,
            matched_policy: None,
        })
    }
}

impl CompiledPolicy {
    fn new(policy: FolderPolicy) -> Result<Self, FolderPolicyError> {
        let required_children = policy
            .matcher
            .required_child_globs
            .iter()
            .map(|pattern| Glob::new(pattern).map(|glob| glob.compile_matcher()))
            .collect::<Result<Vec<_>, _>>()?;
        let default_includes = compile_set(&policy.behavior.default_include_globs)?;
        let navigation_hides = compile_set(&policy.behavior.navigation_hide_globs)?;
        Ok(Self {
            policy,
            required_children,
            default_includes,
            navigation_hides,
        })
    }

    fn matches_structure(&self, structure: &FolderStructureSnapshot) -> bool {
        self.required_children.iter().all(|required| {
            structure
                .children
                .iter()
                .any(|child| required.is_match(child))
        })
    }

    fn classify(&self, asset_path: &Path) -> PolicyDecision {
        let group_path = asset_path
            .components()
            .take(self.policy.group_depth)
            .collect::<PathBuf>();
        let relative = asset_path.strip_prefix(&group_path).unwrap_or(asset_path);
        let relative_parent = relative.parent().unwrap_or(Path::new(""));
        let navigation_path = if self.navigation_hides.is_match(relative_parent) {
            group_path.clone()
        } else {
            group_path.join(relative_parent)
        };
        let visible_by_default = self.default_includes.is_empty()
            || self.default_includes.is_match(relative)
            || self.default_includes.is_match(relative_parent);
        let path_rating = self
            .policy
            .behavior
            .path_rating
            .as_ref()
            .and_then(|rule| rating_from_path(relative_parent, rule));

        PolicyDecision {
            group_path,
            navigation_path,
            visible_by_default,
            path_rating,
            matched_policy: Some(self.policy.name.clone()),
        }
    }
}

fn compile_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    builder.build()
}

fn rating_from_path(path: &Path, rule: &PathRatingRule) -> Option<u8> {
    path.components().find_map(|component| {
        let value = component.as_os_str().to_str()?;
        let number = value
            .strip_suffix(&rule.segment_suffix)?
            .parse::<u8>()
            .ok()?;
        (number <= 5).then_some(number)
    })
}
