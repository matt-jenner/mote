use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderPolicy {
    pub name: String,
    pub group_depth: usize,
    pub matcher: StructureMatcher,
    pub behavior: PolicyBehavior,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StructureMatcher {
    pub required_child_globs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyBehavior {
    pub default_include_globs: Vec<String>,
    pub navigation_hide_globs: Vec<String>,
    pub path_rating: Option<PathRatingRule>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PathRatingRule {
    pub segment_suffix: String,
}
