use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context, Result, anyhow, ensure};
use directories::BaseDirs;
use shared::{AppConfig, MODEL_CATALOG, ModelCatalogEntry, model_catalog_entry};

use crate::command::{AppCommand, DownloadId};
use crate::models::downloader::{self, DownloadHandle};
use crate::models::inventory::{
    load_ready_model_ids, mark_model_ready, model_path, partial_model_path,
    remove_model_from_inventory, remove_orphan_partials,
};
use crate::models::view_model::{ModelRowState, ModelStatus, referenced_models};
use tracing::warn;

#[derive(Debug)]
pub struct ModelStore {
    root: PathBuf,
    ready_model_ids: RefCell<HashSet<String>>,
}

impl ModelStore {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to resolve user data directory")?;
        let root = base_dirs.data_dir().join("myapp/models");
        if let Err(error) = remove_orphan_partials(&root) {
            warn!(?error, root = %root.display(), "failed to remove orphan model partials");
        }
        let ready_model_ids = load_ready_model_ids(&root);
        Ok(Self {
            root,
            ready_model_ids: RefCell::new(ready_model_ids),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn model_path(&self, entry: ModelCatalogEntry) -> PathBuf {
        model_path(&self.root, entry)
    }

    pub fn model_path_for_id(&self, model_id: &str) -> Option<PathBuf> {
        model_catalog_entry(model_id).map(|entry| self.model_path(entry))
    }

    pub fn ready_model_ids(&self) -> HashSet<String> {
        self.ready_model_ids.borrow().clone()
    }

    pub fn mark_ready(&self, model_id: &str) -> Result<HashSet<String>> {
        let entry =
            model_catalog_entry(model_id).ok_or_else(|| anyhow!("unknown model id {model_id}"))?;
        let ready_model_ids = mark_model_ready(&self.root, entry)?;
        ensure!(
            ready_model_ids.contains(model_id),
            "model {model_id} was written to inventory but did not load as ready"
        );
        self.ready_model_ids.replace(ready_model_ids.clone());
        Ok(ready_model_ids)
    }

    pub fn row_states(
        &self,
        config: &AppConfig,
        downloads: &HashMap<String, ModelStatus>,
    ) -> Vec<ModelRowState> {
        let referenced = referenced_models(config);
        let ready_model_ids = self.ready_model_ids.borrow();
        MODEL_CATALOG
            .iter()
            .filter_map(|entry| model_catalog_entry(entry.id))
            .map(|entry| {
                let status = downloads.get(entry.id).cloned().unwrap_or_else(|| {
                    if ready_model_ids.contains(entry.id) {
                        ModelStatus::Ready
                    } else {
                        ModelStatus::NotInstalled
                    }
                });
                ModelRowState {
                    entry,
                    status,
                    referenced: referenced.contains(entry.id),
                }
            })
            .collect()
    }

    pub fn start_download(
        &self,
        download_id: DownloadId,
        model_id: String,
        command_tx: mpsc::Sender<AppCommand>,
    ) -> DownloadHandle {
        downloader::start_download(&self.root, download_id, model_id, command_tx)
    }

    pub fn delete_model(&self, model_id: &str, config: &AppConfig) -> Result<()> {
        if referenced_models(config).contains(model_id) {
            return Err(anyhow!("model {model_id} is still referenced by settings"));
        }
        let entry =
            model_catalog_entry(model_id).ok_or_else(|| anyhow!("unknown model id {model_id}"))?;
        let path = self.model_path(entry);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to delete model {}", path.display()))?;
        }
        let partial = partial_model_path(&self.root, entry);
        if partial.exists() {
            fs::remove_file(&partial)
                .with_context(|| format!("failed to delete partial model {}", partial.display()))?;
        }
        let ready_model_ids = remove_model_from_inventory(&self.root, model_id)?;
        self.ready_model_ids.replace(ready_model_ids);
        Ok(())
    }
}
