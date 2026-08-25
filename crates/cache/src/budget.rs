use std::path::Path;

use photo_catalog::Catalog;
use photo_domain::FolderGroupId;

use crate::{CacheError, EvictionPlanner, ProtectedGroups};

const GIB: u64 = 1024 * 1024 * 1024;
const MAX: u64 = 100 * GIB;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheBudget {
    limit: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BudgetPreparation {
    pub evicted_groups: Vec<FolderGroupId>,
    pub reclaimed_bytes: u64,
}

impl CacheBudget {
    pub fn automatic(cache_root: &Path) -> Result<Self, CacheError> {
        Ok(Self::from_total_space(fs2::total_space(cache_root)?))
    }

    pub const fn from_total_space(total_space: u64) -> Self {
        Self {
            limit: if total_space / 10 < MAX {
                total_space / 10
            } else {
                MAX
            },
        }
    }

    pub const fn limit_bytes(self) -> u64 {
        self.limit
    }

    pub fn prepare_write(
        self,
        catalog: &mut Catalog,
        cache_root: &Path,
        active_group: FolderGroupId,
        estimated_bytes: u64,
        protected: &ProtectedGroups,
    ) -> Result<BudgetPreparation, CacheError> {
        let used = catalog
            .all_derivatives()?
            .into_iter()
            .filter(|d| !d.durable)
            .try_fold(0_u64, |sum, d| {
                sum.checked_add(d.size_bytes)
                    .ok_or(CacheError::SizeOutOfRange)
            })?;
        let needed = used
            .checked_add(estimated_bytes)
            .ok_or(CacheError::SizeOutOfRange)?
            .saturating_sub(self.limit);
        if needed == 0 {
            return Ok(BudgetPreparation::default());
        }
        protected.protect(active_group)?;
        let result = (|| {
            let plan = EvictionPlanner::plan(catalog, needed, protected)?;
            let groups = plan.groups.clone();
            let reclaimed = EvictionPlanner::execute(catalog, cache_root, &plan, protected)?;
            Ok::<_, CacheError>((groups, reclaimed))
        })();
        protected.unprotect(active_group)?;
        let (groups, reclaimed) = result?;
        Ok(BudgetPreparation {
            evicted_groups: groups,
            reclaimed_bytes: reclaimed,
        })
    }
}
