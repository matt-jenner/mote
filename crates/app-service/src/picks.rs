use std::collections::HashMap;

use photo_catalog::{Catalog, PhotoPickRecord};
use photo_domain::{AssetId, FolderGroupId, GalleryScope};

use crate::service::{SelectionToken, wall_assets_with_derivatives};
use crate::{
    AppService, AppServiceError, DerivativeRequest, OrderState, PickItem, PickListSnapshot,
    PickReference,
};

pub(crate) fn parse_pick_id(value: &str) -> Result<uuid::Uuid, AppServiceError> {
    uuid::Uuid::parse_str(value).map_err(|_| AppServiceError::InvalidAssetId)
}

pub(crate) fn parse_pick_asset_ids(ids: &[String]) -> Result<Vec<AssetId>, AppServiceError> {
    if ids.len() > 250 {
        return Err(AppServiceError::InvalidLimit);
    }
    ids.iter()
        .map(|id| parse_pick_id(id).map(AssetId::from_uuid))
        .collect()
}

fn authorize(
    catalog: &Catalog,
    asset: AssetId,
    group: FolderGroupId,
) -> Result<(), AppServiceError> {
    if catalog
        .wall_records_for_assets_scoped(group, GalleryScope::IncludeSubfolders, &[asset])?
        .is_empty()
    {
        return Err(AppServiceError::ForeignAsset);
    }
    Ok(())
}

fn snapshot(catalog: &Catalog) -> Result<PickListSnapshot, AppServiceError> {
    let picks = catalog.list_photo_picks()?;
    let saved = catalog.list_saved_folders()?;
    let mut hydrated = HashMap::new();
    let mut groups: HashMap<FolderGroupId, Vec<AssetId>> = HashMap::new();
    for pick in &picks {
        groups
            .entry(pick.folder_group_id)
            .or_default()
            .push(pick.asset_id);
    }
    for (group_id, ids) in groups {
        let group = catalog
            .folder_group(group_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let library = catalog
            .find_library(group.library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let label = saved
            .iter()
            .find(|entry| entry.folder_group_id == group_id)
            .and_then(|entry| entry.custom_label.clone())
            .unwrap_or_else(|| {
                group
                    .relative_path
                    .to_path_buf()
                    .ok()
                    .and_then(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or(library.display_name)
            });
        let order = if catalog.has_completed_generation_for_group(group.library_id, group_id)? {
            OrderState::Settled
        } else {
            OrderState::Provisional
        };
        let records = catalog.wall_records_for_assets_scoped(
            group_id,
            GalleryScope::IncludeSubfolders,
            &ids,
        )?;
        let assets = wall_assets_with_derivatives(catalog, &records, order)?
            .into_iter()
            .map(|asset| (asset.id.clone(), asset))
            .collect::<HashMap<_, _>>();
        hydrated.insert(group_id, (label, assets));
    }
    let items = picks
        .into_iter()
        .map(|pick| {
            let (label, assets) = &hydrated[&pick.folder_group_id];
            let id = pick.asset_id.as_uuid().to_string();
            PickItem {
                asset: assets.get(&id).cloned(),
                reference: PickReference {
                    asset_id: id,
                    source_folder_id: pick.folder_group_id.as_uuid().to_string(),
                    source_label: label.clone(),
                },
            }
        })
        .collect();
    Ok(PickListSnapshot {
        revision: catalog.photo_pick_revision()?,
        items,
    })
}

impl AppService {
    pub fn list_photo_picks(&self) -> Result<PickListSnapshot, AppServiceError> {
        snapshot(self.state()?.libraries.catalog())
    }

    pub fn add_photo_pick(
        &self,
        asset_id: &str,
        source_folder_id: &str,
    ) -> Result<PickListSnapshot, AppServiceError> {
        let asset = AssetId::from_uuid(parse_pick_id(asset_id)?);
        let group = FolderGroupId::from_uuid(parse_pick_id(source_folder_id)?);
        let mut state = self.state()?;
        authorize(state.libraries.catalog(), asset, group)?;
        state.libraries.catalog_mut().add_photo_pick(asset, group)?;
        snapshot(state.libraries.catalog())
    }

    pub fn remove_photo_pick(&self, asset_id: &str) -> Result<PickListSnapshot, AppServiceError> {
        let asset = AssetId::from_uuid(parse_pick_id(asset_id)?);
        let mut state = self.state()?;
        state.libraries.catalog_mut().remove_photo_pick(asset)?;
        snapshot(state.libraries.catalog())
    }

    /// Returns the current empty snapshot. Callers retain prior references for Undo.
    pub fn clear_photo_picks(&self) -> Result<PickListSnapshot, AppServiceError> {
        let mut state = self.state()?;
        state.libraries.catalog_mut().clear_photo_picks()?;
        snapshot(state.libraries.catalog())
    }

    pub fn restore_photo_picks(
        &self,
        references: &[PickReference],
    ) -> Result<PickListSnapshot, AppServiceError> {
        if references.len() > 250 {
            return Err(AppServiceError::InvalidLimit);
        }
        let parsed = references
            .iter()
            .map(|reference| {
                Ok((
                    AssetId::from_uuid(parse_pick_id(&reference.asset_id)?),
                    FolderGroupId::from_uuid(parse_pick_id(&reference.source_folder_id)?),
                ))
            })
            .collect::<Result<Vec<_>, AppServiceError>>()?;
        let mut state = self.state()?;
        let catalog = state.libraries.catalog();
        let mut records = Vec::new();
        for (asset_id, folder_group_id) in parsed {
            // Hard deletion can happen between Clear and Undo. Foreign keys cannot
            // retain that entry, but other cleared picks can still be restored.
            if catalog.find_asset(asset_id)?.is_none()
                || catalog.folder_group(folder_group_id)?.is_none()
            {
                continue;
            }
            authorize(catalog, asset_id, folder_group_id)?;
            records.push(PhotoPickRecord {
                asset_id,
                folder_group_id,
                position: records.len() as u64,
            });
        }
        state
            .libraries
            .catalog_mut()
            .restore_photo_picks(&records)?;
        snapshot(state.libraries.catalog())
    }

    pub async fn request_pick_derivatives(
        &self,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        let ids = parse_pick_asset_ids(&request.asset_ids)?;
        if ids.is_empty() {
            return Err(AppServiceError::InvalidLimit);
        }
        let grouped = {
            let state = self.state()?;
            let catalog = state.libraries.catalog();
            let picks = catalog
                .list_photo_picks()?
                .into_iter()
                .map(|pick| (pick.asset_id, pick.folder_group_id))
                .collect::<HashMap<_, _>>();
            let mut groups: HashMap<FolderGroupId, (SelectionToken, Vec<String>)> = HashMap::new();
            for id in ids {
                let group_id = *picks.get(&id).ok_or(AppServiceError::ForeignAsset)?;
                authorize(catalog, id, group_id)?;
                let group = catalog
                    .folder_group(group_id)?
                    .ok_or(AppServiceError::UnknownAsset)?;
                groups
                    .entry(group_id)
                    .or_insert_with(|| {
                        (
                            SelectionToken {
                                library_id: group.library_id,
                                group_id,
                                epoch: 0,
                            },
                            Vec::new(),
                        )
                    })
                    .1
                    .push(id.as_uuid().to_string());
            }
            groups
        };
        for (_, (token, asset_ids)) in grouped {
            let selection = self.pick_gallery.selection_from_token(token)?;
            self.pick_gallery
                .request_derivatives(
                    &selection,
                    GalleryScope::IncludeSubfolders,
                    DerivativeRequest {
                        asset_ids,
                        kind: request.kind,
                        priority: request.priority,
                    },
                )
                .await?;
        }
        Ok(())
    }
}
