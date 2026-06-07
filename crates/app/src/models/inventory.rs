use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shared::{ModelCatalogEntry, model_catalog_entry};
use tracing::{info, warn};

const INVENTORY_FILE_NAME: &str = "inventory.toml";
const INVENTORY_SCHEMA_VERSION: u32 = 2;
const BROKEN_SIZE_CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelInventory {
    schema_version: u32,
    ready: Vec<ModelInventoryEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelInventoryEntry {
    id: String,
    filename: String,
    size_bytes: u64,
    sha1: String,
}

pub fn load_ready_model_ids(root: &Path) -> HashSet<String> {
    let inventory = load_inventory(root).unwrap_or_else(|error| {
        warn!(?error, "failed to load model inventory cache");
        ModelInventory::default()
    });

    inventory
        .ready
        .into_iter()
        .filter_map(|entry| {
            let catalog_entry = model_catalog_entry(&entry.id)?;
            ready_entry_is_valid(root, &entry, catalog_entry).then(|| entry.id.to_string())
        })
        .collect()
}

pub fn mark_model_ready(root: &Path, entry: ModelCatalogEntry) -> Result<HashSet<String>> {
    let mut inventory = load_inventory(root).unwrap_or_default();
    inventory.schema_version = INVENTORY_SCHEMA_VERSION;
    let ready_entry = ModelInventoryEntry::from_verified_file(root, entry)?;
    info!(
        model_id = %ready_entry.id,
        size_bytes = ready_entry.size_bytes,
        "marking model ready in inventory"
    );
    inventory.ready.retain(|cached| cached.id != ready_entry.id);
    inventory.ready.push(ready_entry);
    save_inventory(root, &inventory)?;
    Ok(load_ready_model_ids(root))
}

pub fn remove_model_from_inventory(root: &Path, model_id: &str) -> Result<HashSet<String>> {
    let mut inventory = load_inventory(root).unwrap_or_default();
    inventory.schema_version = INVENTORY_SCHEMA_VERSION;
    inventory.ready.retain(|cached| cached.id != model_id);
    save_inventory(root, &inventory)?;
    Ok(load_ready_model_ids(root))
}

pub fn model_path(root: &Path, entry: ModelCatalogEntry) -> PathBuf {
    root.join(entry.filename)
}

pub fn partial_model_path(root: &Path, entry: ModelCatalogEntry) -> PathBuf {
    model_path(root, entry).with_extension("bin.part")
}

fn load_inventory(root: &Path) -> Result<ModelInventory> {
    let path = inventory_path(root);
    if !path.exists() {
        return Ok(ModelInventory::default());
    }

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read model inventory {}", path.display()))?;
    let inventory: ModelInventory = toml::from_str(&text)
        .with_context(|| format!("failed to parse model inventory {}", path.display()))?;
    match inventory.schema_version {
        INVENTORY_SCHEMA_VERSION => Ok(inventory),
        BROKEN_SIZE_CACHE_SCHEMA_VERSION => repair_broken_size_inventory(root, inventory),
        version => {
            warn!(
                version,
                expected = INVENTORY_SCHEMA_VERSION,
                "ignoring unsupported model inventory schema"
            );
            Ok(ModelInventory::default())
        }
    }
}

fn save_inventory(root: &Path, inventory: &ModelInventory) -> Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create model directory {}", root.display()))?;
    let path = inventory_path(root);
    let text = toml::to_string_pretty(inventory).context("failed to serialize model inventory")?;
    fs::write(&path, text)
        .with_context(|| format!("failed to write model inventory {}", path.display()))
}

fn inventory_path(root: &Path) -> PathBuf {
    root.join(INVENTORY_FILE_NAME)
}

fn repair_broken_size_inventory(root: &Path, inventory: ModelInventory) -> Result<ModelInventory> {
    let mut repaired = ModelInventory::default();
    for entry in inventory.ready {
        let Some(catalog_entry) = model_catalog_entry(&entry.id) else {
            warn!(model_id = %entry.id, "skipping unknown cached model entry");
            continue;
        };
        if !inventory_entry_matches_catalog_identity(&entry, catalog_entry) {
            warn!(
                model_id = %entry.id,
                "skipping cached model entry with stale catalog identity"
            );
            continue;
        }
        let Ok(size_bytes) = actual_model_size(root, catalog_entry) else {
            warn!(
                model_id = %entry.id,
                filename = catalog_entry.filename,
                "skipping cached model entry because final file is missing"
            );
            continue;
        };

        info!(
            model_id = %entry.id,
            old_size_bytes = entry.size_bytes,
            size_bytes,
            "repairing model inventory entry with actual file size"
        );
        repaired.ready.push(ModelInventoryEntry {
            id: entry.id,
            filename: entry.filename,
            size_bytes,
            sha1: entry.sha1,
        });
    }

    save_inventory(root, &repaired)?;
    Ok(repaired)
}

fn inventory_entry_matches_catalog_identity(
    entry: &ModelInventoryEntry,
    catalog_entry: ModelCatalogEntry,
) -> bool {
    entry.filename == catalog_entry.filename && entry.sha1 == catalog_entry.sha1
}

fn ready_entry_is_valid(
    root: &Path,
    entry: &ModelInventoryEntry,
    catalog_entry: ModelCatalogEntry,
) -> bool {
    if !inventory_entry_matches_catalog_identity(entry, catalog_entry) {
        warn!(
            model_id = %entry.id,
            "model inventory entry does not match current catalog identity"
        );
        return false;
    }

    match actual_model_size(root, catalog_entry) {
        Ok(actual_size) if actual_size == entry.size_bytes => true,
        Ok(actual_size) => {
            warn!(
                model_id = %entry.id,
                recorded_size_bytes = entry.size_bytes,
                actual_size_bytes = actual_size,
                "model inventory entry size does not match local file"
            );
            false
        }
        Err(error) => {
            warn!(
                ?error,
                model_id = %entry.id,
                filename = catalog_entry.filename,
                "model inventory entry points to missing local file"
            );
            false
        }
    }
}

fn actual_model_size(root: &Path, entry: ModelCatalogEntry) -> Result<u64> {
    let path = model_path(root, entry);
    Ok(fs::metadata(&path)
        .with_context(|| format!("failed to stat model file {}", path.display()))?
        .len())
}

impl ModelInventoryEntry {
    fn from_verified_file(root: &Path, entry: ModelCatalogEntry) -> Result<Self> {
        Ok(Self {
            id: entry.id.to_string(),
            filename: entry.filename.to_string(),
            size_bytes: actual_model_size(root, entry)?,
            sha1: entry.sha1.to_string(),
        })
    }
}

impl Default for ModelInventory {
    fn default() -> Self {
        Self {
            schema_version: INVENTORY_SCHEMA_VERSION,
            ready: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn inventory_cache_marks_model_ready_with_actual_file_size() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        let actual_size = entry.size_bytes - 1024;
        create_sparse_model(&root, entry, actual_size);

        let ready = mark_model_ready(&root, entry).unwrap();

        assert!(ready.contains(entry.id));
        assert!(load_ready_model_ids(&root).contains(entry.id));
        let inventory = load_inventory(&root).unwrap();
        assert_eq!(inventory.schema_version, INVENTORY_SCHEMA_VERSION);
        assert_eq!(inventory.ready[0].size_bytes, actual_size);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_cached_file_is_not_ready() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();

        assert!(mark_model_ready(&root, entry).is_err());
        save_inventory(
            &root,
            &ModelInventory {
                schema_version: INVENTORY_SCHEMA_VERSION,
                ready: vec![ModelInventoryEntry {
                    id: entry.id.to_string(),
                    filename: entry.filename.to_string(),
                    size_bytes: entry.size_bytes,
                    sha1: entry.sha1.to_string(),
                }],
            },
        )
        .unwrap();

        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn changed_file_size_after_mark_ready_is_not_ready() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        let actual_size = entry.size_bytes - 2048;
        create_sparse_model(&root, entry, actual_size);

        mark_model_ready(&root, entry).unwrap();
        File::create(model_path(&root, entry))
            .unwrap()
            .set_len(actual_size + 1)
            .unwrap();

        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_model_updates_inventory_cache() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        create_sparse_model(&root, entry, entry.size_bytes);
        mark_model_ready(&root, entry).unwrap();

        let ready = remove_model_from_inventory(&root, entry.id).unwrap();

        assert!(!ready.contains(entry.id));
        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repairs_broken_v1_inventory_cache_with_actual_file_size() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        let actual_size = entry.size_bytes - 4096;
        create_sparse_model(&root, entry, actual_size);
        save_inventory(
            &root,
            &ModelInventory {
                schema_version: BROKEN_SIZE_CACHE_SCHEMA_VERSION,
                ready: vec![ModelInventoryEntry {
                    id: entry.id.to_string(),
                    filename: entry.filename.to_string(),
                    size_bytes: entry.size_bytes,
                    sha1: entry.sha1.to_string(),
                }],
            },
        )
        .unwrap();

        let ready = load_ready_model_ids(&root);

        assert!(ready.contains(entry.id));
        let inventory = load_inventory(&root).unwrap();
        assert_eq!(inventory.schema_version, INVENTORY_SCHEMA_VERSION);
        assert_eq!(inventory.ready[0].size_bytes, actual_size);
        let _ = fs::remove_dir_all(root);
    }

    fn temp_model_root() -> PathBuf {
        let id = TEST_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("myapp-model-inventory-test-{id}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn create_sparse_model(root: &Path, entry: ModelCatalogEntry, size_bytes: u64) {
        fs::create_dir_all(root).unwrap();
        File::create(model_path(root, entry))
            .unwrap()
            .set_len(size_bytes)
            .unwrap();
    }
}
