use crate::service::{SelectionToken, ServiceState};
use crate::{
    AccessReply, AppService, AppServiceError, BootstrapState, FolderAccess, FolderAccessKey,
    FolderAccessState, FolderAccessTarget, FolderProbeOutcome, SavedFolder, SavedFolderSnapshot,
};
use photo_catalog::CatalogError;
use std::sync::atomic::Ordering;

fn invalid() -> AppServiceError {
    AppServiceError::UnknownAsset
}
fn id(value: &str) -> Result<uuid::Uuid, AppServiceError> {
    uuid::Uuid::parse_str(value).map_err(|_| invalid())
}

fn natural_order(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    let mut a = left.chars().peekable();
    let mut b = right.chars().peekable();
    loop {
        match (a.peek(), b.peek()) {
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut x = String::new();
                let mut y = String::new();
                while a.peek().is_some_and(char::is_ascii_digit) {
                    x.push(a.next().unwrap());
                }
                while b.peek().is_some_and(char::is_ascii_digit) {
                    y.push(b.next().unwrap());
                }
                let x = x.trim_start_matches('0');
                let y = y.trim_start_matches('0');
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if !order.is_eq() {
                    return order;
                }
            }
            _ => {
                let order = a.next().cmp(&b.next());
                if !order.is_eq() {
                    return order;
                }
                if a.peek().is_none() && b.peek().is_none() {
                    return order;
                }
            }
        }
    }
}

fn saved_folder_order(left: &SavedFolder, right: &SavedFolder) -> std::cmp::Ordering {
    natural_order(
        left.custom_label.as_deref().unwrap_or(&left.name),
        right.custom_label.as_deref().unwrap_or(&right.name),
    )
    .then_with(|| {
        left.display_path
            .encode_utf16()
            .cmp(right.display_path.encode_utf16())
    })
    .then_with(|| left.id.encode_utf16().cmp(right.id.encode_utf16()))
}

impl AppService {
    pub(crate) fn saved_folders_locked(
        state: &ServiceState,
    ) -> Result<SavedFolderSnapshot, AppServiceError> {
        let catalog = state.libraries.catalog();
        let active = catalog.load_app_state()?.active_selection;
        let mut result = SavedFolderSnapshot {
            revision: state.folder_revision,
            has_opened_folder: catalog.has_opened_folder()?,
            ..Default::default()
        };
        for saved in catalog.list_saved_folders()? {
            let group = catalog
                .folder_group(saved.folder_group_id)?
                .ok_or_else(invalid)?;
            let library = catalog
                .find_library(group.library_id)?
                .ok_or_else(invalid)?;
            let relative = group
                .relative_path
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            let folder_id = group.id.as_uuid().to_string();
            if active.as_ref().is_some_and(|a| {
                a.library_id == group.library_id && a.relative_folder == group.relative_path
            }) {
                result.active_entry_id = Some(saved.id.to_string());
            }
            result.entries.push(SavedFolder {
                id: saved.id.to_string(),
                folder_id: folder_id.clone(),
                name: relative
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(library.display_name),
                display_path: (if relative.as_os_str().is_empty() {
                    root
                } else {
                    root.join(relative)
                })
                .to_string_lossy()
                .into_owned(),
                custom_label: saved.custom_label,
            });
            result.access.insert(
                folder_id.clone(),
                state
                    .folder_status
                    .get(&folder_id)
                    .cloned()
                    .unwrap_or(FolderAccess {
                        folder_id,
                        state: FolderAccessState::Unknown,
                        generation: 0,
                        retry_after_ms: 0,
                    }),
            );
        }
        Ok(result)
    }

    fn saved_target(
        &self,
        entry: &str,
    ) -> Result<(photo_domain::FolderGroupId, FolderAccessTarget), AppServiceError> {
        let state = self.state()?;
        let catalog = state.libraries.catalog();
        let saved = catalog.find_saved_folder(id(entry)?)?.ok_or_else(invalid)?;
        let group = catalog
            .folder_group(saved.folder_group_id)?
            .ok_or_else(invalid)?;
        let library = catalog
            .find_library(group.library_id)?
            .ok_or_else(invalid)?;
        Ok((
            group.id,
            FolderAccessTarget {
                key: FolderAccessKey {
                    library_id: group.library_id,
                    relative: group.relative_path,
                },
                root: library
                    .canonical_root_key
                    .to_path_buf()
                    .map_err(|e| CatalogError::InvalidData(e.to_string()))?,
            },
        ))
    }

    async fn check_saved_folder(&self, entry: &str) -> Result<AccessReply, AppServiceError> {
        let (group, target) = self.saved_target(entry)?;
        let library = target.key.library_id;
        let root_selection = target.key.relative.as_bytes().is_empty();
        let reply = self.folder_access.check_with_root(target).await;
        let access = FolderAccess::from_reply(group.as_uuid().to_string(), &reply);
        let mut state = self.state()?;
        if state
            .libraries
            .catalog()
            .find_saved_folder(id(entry)?)?
            .is_some()
        {
            if matches!(
                &reply,
                AccessReply::Complete {
                    outcome,
                    ..
                } if matches!(outcome, FolderProbeOutcome::Missing | FolderProbeOutcome::Unreadable | FolderProbeOutcome::RootOffline)
            ) {
                if root_selection {
                    state.libraries.catalog_mut().mark_root_offline(library)?;
                } else {
                    state
                        .libraries
                        .catalog_mut()
                        .mark_group_offline(library, group)?;
                }
            }
            let old = state.folder_status.get(&access.folder_id);
            if old.is_none_or(|old| old.generation <= access.generation) {
                state.folder_revision += 1;
                state.folder_status.insert(access.folder_id.clone(), access);
            }
        }
        Ok(reply)
    }

    pub async fn check_saved_folders(
        &self,
        entries: &[String],
    ) -> Result<SavedFolderSnapshot, AppServiceError> {
        // Each caller has a bounded batch; coordinator shares probes across callers.
        let mut checks = tokio::task::JoinSet::new();
        for entry in entries.iter().take(256) {
            if self.saved_target(entry).is_ok() {
                let service = self.clone();
                let entry = entry.clone();
                checks.spawn(async move { service.check_saved_folder(&entry).await });
            }
        }
        while checks.join_next().await.is_some() {}
        Ok(self.bootstrap()?.saved_folders)
    }

    pub async fn desktop_bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        let (bootstrap, verification) = {
            let _transition = self
                .selection_transition
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            let mut state = self.state()?;
            let saved = Self::saved_folders_locked(&state)?;
            if let Some(first) = saved.entries.iter().min_by(|a, b| saved_folder_order(a, b)) {
                let group = photo_domain::FolderGroupId::from_uuid(id(&first.folder_id)?);
                self.invalidate_folder_selection(&mut state)?;
                state
                    .libraries
                    .catalog_mut()
                    .save_and_activate_folder(group)?;
                self.protected_groups.protect(group)?;
                state.protected_group = Some(group);
                state.folder_revision += 1;
            }
            (
                Self::bootstrap_locked(&state)?,
                Self::restore_verification_locked(&state)?,
            )
        };
        self.spawn_restore_verification(verification);
        Ok(bootstrap)
    }

    pub(crate) fn restore_verification_locked(
        state: &ServiceState,
    ) -> Result<Option<(String, SelectionToken)>, AppServiceError> {
        let snapshot = Self::saved_folders_locked(state)?;
        let Some(entry) = snapshot
            .entries
            .iter()
            .find(|entry| Some(&entry.id) == snapshot.active_entry_id.as_ref())
        else {
            return Ok(None);
        };
        let group = state
            .libraries
            .catalog()
            .folder_group(photo_domain::FolderGroupId::from_uuid(
                id(&entry.folder_id)?,
            ))?
            .ok_or_else(invalid)?;
        Ok(Some((
            entry.id.clone(),
            SelectionToken {
                library_id: group.library_id,
                group_id: group.id,
                epoch: state.selection_epoch,
            },
        )))
    }

    pub(crate) fn spawn_restore_verification(
        &self,
        verification: Option<(String, SelectionToken)>,
    ) {
        let Some((entry, selection)) = verification else {
            return;
        };
        let service = self.clone();
        tokio::spawn(async move {
            let available = matches!(
                service.check_saved_folder(&entry).await,
                Ok(AccessReply::Complete {
                    outcome: FolderProbeOutcome::Available(_),
                    ..
                })
            );
            if available {
                let _ = service.start_selected_scan(selection).await;
            }
        });
    }

    pub async fn checked_bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        self.desktop_bootstrap().await
    }

    pub fn rename_saved_folder(
        &self,
        entry: &str,
        label: Option<&str>,
    ) -> Result<BootstrapState, AppServiceError> {
        let mut state = self.state()?;
        let saved = state
            .libraries
            .catalog()
            .find_saved_folder(id(entry)?)?
            .ok_or_else(invalid)?;
        if state
            .folder_status
            .get(&saved.folder_group_id.as_uuid().to_string())
            .is_some_and(|a| {
                a.state != FolderAccessState::Available && a.state != FolderAccessState::Unknown
            })
        {
            return Err(invalid());
        }
        state
            .libraries
            .catalog_mut()
            .rename_saved_folder(id(entry)?, label)?;
        state.folder_revision += 1;
        Self::bootstrap_locked(&state)
    }

    fn invalidate_folder_selection(&self, state: &mut ServiceState) -> Result<(), AppServiceError> {
        let sequence = self
            .selection_request_sequence
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        self.latest_validated_selection
            .fetch_max(sequence, Ordering::SeqCst);
        state.selection_epoch = state.selection_epoch.wrapping_add(1).max(1);
        if let Some(group) = state.protected_group.take() {
            self.protected_groups.unprotect(group)?;
        }
        state.published_wall_cache_warning = None;
        state.published_screen_cache_warning = None;
        Ok(())
    }

    pub fn remove_saved_folder(&self, entry: &str) -> Result<BootstrapState, AppServiceError> {
        let _transition = self
            .selection_transition
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let mut state = self.state()?;
        let active = state
            .libraries
            .catalog_mut()
            .remove_saved_folder(id(entry)?)?;
        state.folder_revision += 1;
        let cancelled = if active {
            self.invalidate_folder_selection(&mut state)?;
            state.active_scan.take()
        } else {
            None
        };
        let bootstrap = Self::bootstrap_locked(&state)?;
        drop(state);
        if let Some(scan) = cancelled {
            self.gallery.cancel_runtime_scan(scan.selection.group_id);
        }
        Ok(bootstrap)
    }

    pub fn clear_active_folder(&self) -> Result<BootstrapState, AppServiceError> {
        self.clear_active_folder_if(None)
    }

    fn clear_active_folder_if(
        &self,
        expected: Option<(u64, Option<String>)>,
    ) -> Result<BootstrapState, AppServiceError> {
        let _transition = self
            .selection_transition
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let mut state = self.state()?;
        if let Some((sequence, selection)) = expected {
            let current = Self::bootstrap_locked(&state)?;
            if self.selection_request_sequence.load(Ordering::SeqCst) != sequence
                || current
                    .active_source
                    .as_ref()
                    .map(|s| s.selection_id.clone())
                    != selection
            {
                return Ok(current);
            }
        }
        state.libraries.catalog_mut().set_active_selection(None)?;
        state.folder_revision += 1;
        self.invalidate_folder_selection(&mut state)?;
        let cancelled = state.active_scan.take();
        let bootstrap = Self::bootstrap_locked(&state)?;
        drop(state);
        if let Some(scan) = cancelled {
            self.gallery.cancel_runtime_scan(scan.selection.group_id);
        }
        Ok(bootstrap)
    }

    pub async fn open_folder(
        &self,
        path: std::path::PathBuf,
    ) -> Result<Option<BootstrapState>, AppServiceError> {
        let mut identities = Vec::new();
        for entry in self.bootstrap()?.saved_folders.entries {
            let (_, target) = self.saved_target(&entry.id)?;
            let relative = target
                .key
                .relative
                .to_path_buf()
                .map_err(|e| CatalogError::InvalidData(e.to_string()))?;
            identities.push((entry.id, target.root.join(relative)));
        }
        if let Some((id, _)) = identities.iter().find(|(_, native)| *native == path) {
            return self.activate_saved_folder(id).await;
        }
        let Some(canonical) = self.folder_access.canonical_identity(path.clone()).await else {
            return Ok(None);
        };
        if let Some((id, _)) = identities.iter().find(|(_, native)| *native == canonical) {
            return self.activate_saved_folder(id).await;
        }
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.open_recent(&path))
            .await
            .map_err(|_| AppServiceError::StatePoisoned)?
            .map(Some)
    }

    pub async fn activate_saved_folder(
        &self,
        entry: &str,
    ) -> Result<Option<BootstrapState>, AppServiceError> {
        let request = self
            .selection_request_sequence
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let reply = self.check_saved_folder(entry).await?;
        let AccessReply::Complete {
            outcome: FolderProbeOutcome::Available(proof),
            ..
        } = reply
        else {
            return Ok(None);
        };
        let token = {
            let _transition = self
                .selection_transition
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            if self.selection_request_sequence.load(Ordering::SeqCst) != request {
                return Ok(None);
            }
            let mut state = self.state()?;
            let saved = state
                .libraries
                .catalog()
                .find_saved_folder(id(entry)?)?
                .ok_or_else(invalid)?;
            let group = state
                .libraries
                .catalog()
                .folder_group(saved.folder_group_id)?
                .ok_or_else(invalid)?;
            if group.library_id != proof.key().library_id
                || group.relative_path != proof.key().relative
            {
                return Ok(None);
            }
            self.latest_validated_selection
                .fetch_max(request, Ordering::SeqCst);
            let needs_protection = state.protected_group != Some(group.id);
            if needs_protection {
                self.protected_groups.protect(group.id)?;
            }
            if let Err(error) = state
                .libraries
                .catalog_mut()
                .save_and_activate_folder(group.id)
            {
                if needs_protection {
                    let _ = self.protected_groups.unprotect(group.id);
                }
                return Err(error.into());
            }
            state.folder_revision += 1;
            let previous = state.protected_group.replace(group.id);
            if let Some(previous) = previous.filter(|old| *old != group.id) {
                self.protected_groups.unprotect(previous)?;
            }
            let old_scan = state.active_scan.take();
            state.selection_epoch = state.selection_epoch.wrapping_add(1).max(1);
            state.published_wall_cache_warning = None;
            state.published_screen_cache_warning = None;
            let token = SelectionToken {
                library_id: group.library_id,
                group_id: group.id,
                epoch: state.selection_epoch,
            };
            drop(state);
            if let Some(old) = old_scan {
                self.gallery.cancel_runtime_scan(old.selection.group_id);
            }
            token
        };
        self.coordinator.reset_selection(token).await;
        self.start_selected_scan(token).await?;
        Ok(Some(self.bootstrap()?))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};
    #[tokio::test]
    async fn desktop_bootstrap_verification_cannot_start_a_replacement_selection() {
        let temp = tempfile::tempdir().unwrap();
        let service = AppService::open(crate::AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let mut original = String::new();
        let mut captured = None;
        for name in ["a", "b"] {
            let path = temp.path().join(name);
            std::fs::create_dir(&path).unwrap();
            image::ImageBuffer::from_pixel(12, 8, image::Rgb([10_u8, 20, 30]))
                .save(path.join("photo.jpg"))
                .unwrap();
            let (bootstrap, token) = service.select_recent(&path).unwrap();
            if name == "a" {
                original = bootstrap.saved_folders.active_entry_id.unwrap();
                captured = Some(token);
            }
        }
        let captured = captured.unwrap();
        let mut updates = service.subscribe_wall_updates();
        service.spawn_restore_verification(Some((original.clone(), captured)));
        service.check_saved_folders(&[original]).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), updates.recv())
                .await
                .is_err(),
            "verification for A must not scan the replacement B"
        );
        assert_eq!(
            service
                .bootstrap()
                .unwrap()
                .active_source
                .unwrap()
                .display_name,
            "b"
        );
    }
    #[test]
    fn desktop_bootstrap_order_matches_sidebar_for_duplicate_and_non_ascii_labels() {
        let mut folders = [
            ("two", "Photos", "/photos/2"),
            ("ten", "Photos", "/photos/10"),
            ("accent10", "éclair 10", "/photos/e10"),
            ("accent2", "Éclair 2", "/photos/e2"),
            ("zebra", "Zebra", "/photos/z"),
        ]
        .map(|(id, name, path)| SavedFolder {
            id: id.into(),
            folder_id: id.into(),
            name: name.into(),
            display_path: path.into(),
            custom_label: None,
        });
        folders.sort_by(saved_folder_order);
        assert_eq!(
            folders.map(|f| f.id),
            ["ten", "two", "zebra", "accent2", "accent10"]
        );
    }
    #[test]
    fn desktop_bootstrap_order_compares_text_after_equal_numbers() {
        assert!(natural_order("album 2b", "album 2a").is_gt());
        assert!(natural_order("Album 10", "album 2").is_gt());
    }
    #[tokio::test]
    async fn desktop_bootstrap_indeterminate_probe_keeps_availability() {
        struct Indeterminate(FolderProbeOutcome);
        impl crate::FolderProbe for Indeterminate {
            fn probe(&self, _: &FolderAccessTarget) -> FolderProbeOutcome {
                self.0.clone()
            }
        }
        for outcome in [FolderProbeOutcome::Failed, FolderProbeOutcome::Invalid] {
            let temp = tempfile::tempdir().unwrap();
            let photos = temp.path().join("photos");
            std::fs::create_dir(&photos).unwrap();
            let mut service = AppService::open(crate::AppConfig::new(
                temp.path().join("data"),
                temp.path().join("cache"),
            ))
            .unwrap();
            let saved = service.open_recent(&photos).unwrap();
            let token = service.active_selection_token().unwrap();
            let before = service
                .state()
                .unwrap()
                .libraries
                .catalog()
                .folder_group_recovery_state(token.library_id, token.group_id)
                .unwrap();
            service.folder_access =
                crate::FolderAccessCoordinator::new(Arc::new(Indeterminate(outcome)));
            service
                .check_saved_folders(&[saved.saved_folders.active_entry_id.unwrap()])
                .await
                .unwrap();
            let after = service
                .state()
                .unwrap()
                .libraries
                .catalog()
                .folder_group_recovery_state(token.library_id, token.group_id)
                .unwrap();
            assert_eq!(
                after.requested, before.requested,
                "indeterminate access must not request offline recovery"
            );
        }
    }
    #[tokio::test]
    async fn desktop_bootstrap_returns_while_access_probe_is_blocked() {
        struct Blocked(std::sync::Mutex<std::sync::mpsc::Receiver<()>>);
        impl crate::FolderProbe for Blocked {
            fn probe(&self, _: &FolderAccessTarget) -> FolderProbeOutcome {
                let _ = self.0.lock().unwrap().recv();
                FolderProbeOutcome::Missing
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let photos = temp.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        image::ImageBuffer::from_pixel(12, 8, image::Rgb([40_u8, 20, 5]))
            .save(photos.join("photo.jpg"))
            .unwrap();
        let mut service = AppService::open(crate::AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let mut updates = service.subscribe_wall_updates();
        let saved = service.start_scan(&photos).await.unwrap();
        while !matches!(
            updates.recv().await.unwrap(),
            crate::WallUpdate::MetadataSettled { .. }
        ) {}
        let query = crate::WallQueryRequest {
            cursor: None,
            limit: 100,
            direction: crate::SortDirection::OldestFirst,
        };
        let ids = service
            .query_wall(query.clone())
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|a| a.id)
            .collect();
        service
            .request_derivatives(crate::DerivativeRequest::visible(ids))
            .await
            .unwrap();
        loop {
            let page = service.query_wall(query.clone()).await.unwrap();
            if page.items.len() == 1 && page.items[0].wall_thumbnail.is_some() {
                break;
            }
            updates.recv().await.unwrap();
        }
        let (release, blocked) = std::sync::mpsc::channel();
        service.folder_access =
            crate::FolderAccessCoordinator::new(Arc::new(Blocked(std::sync::Mutex::new(blocked))));
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            service.desktop_bootstrap(),
        )
        .await;
        let bootstrap = result
            .expect("bootstrap must not await the access probe")
            .unwrap();
        assert_eq!(
            bootstrap.saved_folders.active_entry_id,
            saved.saved_folders.active_entry_id
        );
        let page = service.query_wall(query.clone()).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].wall_thumbnail.is_some());
        drop(release);
        service
            .check_saved_folders(&[bootstrap.saved_folders.active_entry_id.unwrap()])
            .await
            .unwrap();
        let page = service.query_wall(query).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].wall_thumbnail.is_some());
    }
    struct Missing(Arc<AtomicUsize>);
    impl crate::FolderProbe for Missing {
        fn probe(&self, _: &FolderAccessTarget) -> FolderProbeOutcome {
            self.0.fetch_add(1, Ordering::SeqCst);
            FolderProbeOutcome::Missing
        }
    }
    #[tokio::test]
    async fn native_picker_alias_reuses_an_existing_failed_check() {
        let temp = tempfile::tempdir().unwrap();
        let photos = temp.path().join("photos");
        std::fs::create_dir(&photos).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&photos, &alias).unwrap();
        let mut service = AppService::open(crate::AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let saved = service.open_recent(&photos).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        service.folder_access =
            crate::FolderAccessCoordinator::new(Arc::new(Missing(calls.clone())));
        assert!(
            service
                .activate_saved_folder(&saved.saved_folders.entries[0].id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(service.open_folder(alias).await.unwrap().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
