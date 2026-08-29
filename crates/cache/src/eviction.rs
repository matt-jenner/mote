use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use photo_catalog::Catalog;
use photo_domain::FolderGroupId;

use crate::CacheError;
use crate::writer::CacheWriter;

#[derive(Debug, Default)]
struct ProtectionState {
    protected: HashMap<FolderGroupId, usize>,
    active_writes: HashMap<FolderGroupId, usize>,
}

#[derive(Clone, Debug, Default)]
pub struct ProtectedGroups {
    state: Arc<Mutex<ProtectionState>>,
}

impl ProtectedGroups {
    pub fn protect(&self, group: FolderGroupId) -> Result<(), CacheError> {
        *self.lock()?.protected.entry(group).or_default() += 1;
        Ok(())
    }

    pub fn unprotect(&self, group: FolderGroupId) -> Result<(), CacheError> {
        let mut state = self.lock()?;
        if let Some(count) = state.protected.get_mut(&group) {
            *count -= 1;
            if *count == 0 {
                state.protected.remove(&group);
            }
        }
        Ok(())
    }

    pub fn begin_write(&self, group: FolderGroupId) -> Result<ActiveWriteGuard, CacheError> {
        let mut state = self.lock()?;
        *state.active_writes.entry(group).or_default() += 1;
        drop(state);
        Ok(ActiveWriteGuard {
            groups: self.clone(),
            group,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, ProtectionState>, CacheError> {
        self.state
            .lock()
            .map_err(|_| CacheError::ProtectionUnavailable)
    }
}

#[derive(Debug)]
pub struct ActiveWriteGuard {
    groups: ProtectedGroups,
    group: FolderGroupId,
}

impl Drop for ActiveWriteGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.groups.state.lock()
            && let Some(count) = state.active_writes.get_mut(&self.group)
        {
            *count -= 1;
            if *count == 0 {
                state.active_writes.remove(&self.group);
            }
        }
    }
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
        if bytes_to_free == 0 {
            return Ok(EvictionPlan::default());
        }
        let protection = protected.lock()?;
        let mut plan = EvictionPlan::default();
        let blocked = protection
            .protected
            .keys()
            .chain(protection.active_writes.keys())
            .copied()
            .collect::<Vec<_>>();
        for group in catalog.cache_eviction_groups_excluding(&blocked)? {
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
        protected: &ProtectedGroups,
    ) -> Result<u64, CacheError> {
        let protection = protected.lock()?;
        if plan.groups.iter().any(|group| {
            protection.protected.contains_key(group) || protection.active_writes.contains_key(group)
        }) {
            return Err(CacheError::GroupBecameProtected);
        }
        let derivatives = catalog.non_durable_derivatives(&plan.groups)?;
        let reclaimable = catalog.reclaimable_derivatives_for_groups(&plan.groups)?;
        let reclaimable_ids = reclaimable
            .iter()
            .map(|derivative| derivative.id)
            .collect::<std::collections::HashSet<_>>();
        let writer = CacheWriter::new(cache_root)?;
        for derivative in &derivatives {
            writer.resolve_checked(&derivative.relative_cache_path)?;
        }
        for derivative in &reclaimable {
            writer.remove_relative_file(&derivative.relative_cache_path)?;
        }
        let ids = derivatives
            .iter()
            .map(|derivative| derivative.id)
            .collect::<Vec<_>>();
        catalog.remove_derivative_group_links(&ids, &plan.groups)?;
        derivatives
            .iter()
            .filter(|derivative| reclaimable_ids.contains(&derivative.id))
            .try_fold(0_u64, |total, item| {
                total
                    .checked_add(item.size_bytes)
                    .ok_or(CacheError::SizeOutOfRange)
            })
    }
}
