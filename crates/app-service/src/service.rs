use std::path::Path;

use photo_cache::{CacheError, CacheWriter};
use photo_catalog::{Catalog, CatalogError, StoredSourceSelection};
use photo_core::{AddLibraryError, LibraryService, LocalStateError, RealSourceFs};
use photo_domain::{Appearance, Availability};

use crate::{AppConfig, BootstrapState, SettingsState, SourceAvailability, SourceSummary};

pub struct AppService {
    libraries: LibraryService<RealSourceFs>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppServiceError {
    #[error("local state setup failed: {0}")]
    LocalState(#[from] LocalStateError),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] CatalogError),
    #[error("cache setup failed: {0}")]
    Cache(#[from] CacheError),
    #[error("library service setup failed: {0}")]
    LibrarySetup(#[source] std::io::Error),
    #[error("folder selection failed: {0}")]
    OpenRecent(#[from] AddLibraryError),
}

impl AppService {
    pub fn open(config: AppConfig) -> Result<Self, AppServiceError> {
        let cataloged_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        config.prepare(&cataloged_roots)?;
        let mut catalog = Catalog::open(&config.catalog_path())?;
        config.validate_source_roots(&cataloged_roots)?;
        CacheWriter::new(config.cache_dir())?.reconcile_catalog(&mut catalog)?;
        let libraries = LibraryService::new(
            catalog,
            RealSourceFs,
            vec![config.data_dir().to_owned(), config.cache_dir().to_owned()],
        )
        .map_err(AppServiceError::LibrarySetup)?;
        Ok(Self { libraries })
    }

    pub fn bootstrap(&self) -> Result<BootstrapState, AppServiceError> {
        let stored = self.libraries.catalog().load_app_state()?;
        let active_source = match stored.active_selection {
            Some(selection) => self
                .libraries
                .catalog()
                .find_library(selection.library_id)?
                .map(|library| SourceSummary {
                    id: library.id.as_uuid().hyphenated().to_string(),
                    display_name: library.display_name,
                    availability: map_availability(library.availability),
                }),
            None => None,
        };
        Ok(BootstrapState {
            settings: SettingsState {
                appearance: stored.appearance,
            },
            active_source,
        })
    }

    pub fn open_recent(&mut self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let selection = self.libraries.open_recent(folder)?;
        self.libraries
            .catalog_mut()
            .set_active_selection(Some(&StoredSourceSelection {
                library_id: selection.library_id,
                relative_folder: selection.relative_folder,
            }))?;
        self.bootstrap()
    }

    pub fn update_appearance(
        &mut self,
        appearance: Appearance,
    ) -> Result<BootstrapState, AppServiceError> {
        self.libraries.catalog_mut().set_appearance(appearance)?;
        self.bootstrap()
    }
}

fn map_availability(availability: Availability) -> SourceAvailability {
    match availability {
        Availability::Available => SourceAvailability::Available,
        Availability::RootOffline => SourceAvailability::RootOffline,
        Availability::Missing => SourceAvailability::Missing,
        Availability::Unreadable => SourceAvailability::Unreadable,
    }
}
