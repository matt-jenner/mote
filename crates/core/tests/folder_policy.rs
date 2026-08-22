use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, NewLibrary};
use photo_core::{FolderPolicyEngine, FolderStructureSnapshot};
use photo_domain::{FolderPolicy, PathRatingRule, PolicyBehavior, StructureMatcher};

fn processed_policy(name: &str) -> FolderPolicy {
    FolderPolicy {
        name: name.to_owned(),
        group_depth: 2,
        matcher: StructureMatcher {
            required_child_globs: vec!["Processed".to_owned()],
        },
        behavior: PolicyBehavior {
            default_include_globs: vec!["Processed/**".to_owned()],
            navigation_hide_globs: vec![
                "Processed/**".to_owned(),
                "RAW/**".to_owned(),
                "Videos/**".to_owned(),
            ],
            path_rating: Some(PathRatingRule {
                segment_suffix: " Stars".to_owned(),
            }),
        },
    }
}

fn snapshot(children: &[&str]) -> FolderStructureSnapshot {
    FolderStructureSnapshot::new(children.iter().copied())
}

#[test]
fn processed_policy_flattens_star_folders_and_derives_fallback_rating() {
    let engine = FolderPolicyEngine::new(vec![processed_policy("Processed first")]).unwrap();
    let structure = snapshot(&["RAW", "Videos", "Processed"]);

    let processed = engine
        .classify(
            Path::new("2026/Walk/Processed/3 Stars/final.jpg"),
            &structure,
        )
        .unwrap();
    assert_eq!(processed.group_path, PathBuf::from("2026/Walk"));
    assert!(processed.visible_by_default);
    assert_eq!(processed.path_rating, Some(3));
    assert_eq!(processed.navigation_path, PathBuf::from("2026/Walk"));

    let raw = engine
        .classify(Path::new("2026/Walk/RAW/source.cr3"), &structure)
        .unwrap();
    assert!(!raw.visible_by_default);
    assert_eq!(raw.path_rating, None);
}

#[test]
fn unmatched_structure_recursively_includes_media() {
    let decision = FolderPolicyEngine::new(vec![processed_policy("Processed first")])
        .unwrap()
        .classify(Path::new("2026/Misc/sub/photo.jpg"), &snapshot(&["sub"]))
        .unwrap();

    assert!(decision.visible_by_default);
    assert_eq!(decision.matched_policy, None);
}

#[test]
fn first_matching_policy_wins() {
    let mut first = processed_policy("First");
    first.behavior.default_include_globs = vec!["RAW/**".to_owned()];
    let second = processed_policy("Second");
    let engine = FolderPolicyEngine::new(vec![first, second]).unwrap();

    let decision = engine
        .classify(
            Path::new("2026/Walk/Processed/final.jpg"),
            &snapshot(&["Processed"]),
        )
        .unwrap();

    assert_eq!(decision.matched_policy.as_deref(), Some("First"));
    assert!(!decision.visible_by_default);
}

#[test]
fn malformed_star_folders_do_not_produce_a_rating() {
    let engine = FolderPolicyEngine::new(vec![processed_policy("Processed first")]).unwrap();
    let structure = snapshot(&["Processed"]);

    for segment in ["Stars", "6 Stars", "3 Starred"] {
        let path = PathBuf::from("2026/Walk/Processed")
            .join(segment)
            .join("final.jpg");
        assert_eq!(
            engine.classify(&path, &structure).unwrap().path_rating,
            None,
            "unexpected rating for {segment}"
        );
    }
}

#[test]
fn policies_round_trip_in_declared_order() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let library_id = {
        let mut catalog = Catalog::open(&path).unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured(
                "Pictures",
                Path::new("/mounted/Pictures"),
            ))
            .unwrap();
        let policies = vec![processed_policy("First"), processed_policy("Second")];
        catalog.save_policies(library.id, &policies).unwrap();
        library.id
    };

    let reopened = Catalog::open(&path).unwrap();
    let loaded = reopened.load_policies(library_id).unwrap();

    assert_eq!(
        loaded
            .iter()
            .map(|policy| policy.name.as_str())
            .collect::<Vec<_>>(),
        vec!["First", "Second"]
    );
    assert_eq!(loaded[0], processed_policy("First"));
    assert_eq!(loaded[1], processed_policy("Second"));
}
