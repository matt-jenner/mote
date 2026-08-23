use std::collections::HashSet;
use std::path::Path;

use photo_catalog::Catalog;
use photo_domain::FolderGroupId;

use crate::CacheError;
use crate::writer::validate_relative;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProtectedGroups {
    pub protected: HashSet<FolderGroupId>,
    pub active_writes: HashSet<FolderGroupId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EvictionPlan {
    pub groups: Vec<FolderGroupId>,
    pub reclaimable_bytes: u64,
}

pub struct EvictionPlanner;

impl EvictionPlanner {
    pub fn plan(
        catalog: &Catalog,
        bytes_to_free: u64,
        protected: &ProtectedGroups,
    ) -> Result<EvictionPlan, CacheError> {
        let mut plan = EvictionPlan::default();
        for group in catalog.cache_eviction_groups()? {
            if protected.protected.contains(&group.id)
                || protected.active_writes.contains(&group.id)
            {
                continue;
            }
            plan.groups.push(group.id);
            plan.reclaimable_bytes = plan
                .reclaimable_bytes
                .checked_add(group.reclaimable_bytes)
                .ok_or(CacheError::SizeOutOfRange)?;
            if plan.reclaimable_bytes >= bytes_to_free {
                break;
            }
        }
        Ok(plan)
    }

    pub fn execute(
        catalog: &mut Catalog,
        cache_root: &Path,
        plan: &EvictionPlan,
    ) -> Result<u64, CacheError> {
        let derivatives = catalog.non_durable_derivatives(&plan.groups)?;
        for derivative in &derivatives {
            validate_relative(&derivative.relative_cache_path)?;
        }
        for derivative in &derivatives {
            let path = cache_root.join(&derivative.relative_cache_path);
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let ids = derivatives
            .iter()
            .map(|derivative| derivative.id)
            .collect::<Vec<_>>();
        catalog.delete_derivatives(&ids)?;
        Ok(derivatives.iter().map(|item| item.size_bytes).sum())
    }
}
