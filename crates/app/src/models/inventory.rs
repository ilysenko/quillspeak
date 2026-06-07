use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shared::{ModelCatalogEntry, model_catalog_entry};
use tracing::warn;

const INVENTORY_FILE_NAME: &str = "inventory.toml";
const INVENTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Deserialize, Serialize)]
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
            inventory_entry_matches_catalog(&entry, catalog_entry)
                .then_some(catalog_entry)
                .filter(|entry| cached_model_file_exists(root, *entry))
                .map(|entry| entry.id.to_string())
        })
        .collect()
}

pub fn mark_model_ready(root: &Path, entry: ModelCatalogEntry) -> Result<HashSet<String>> {
    let mut inventory = load_inventory(root).unwrap_or_default();
    inventory.schema_version = INVENTORY_SCHEMA_VERSION;
    inventory.ready.retain(|cached| cached.id != entry.id);
    inventory.ready.push(ModelInventoryEntry::from(entry));
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
        return Ok(ModelInventory {
            schema_version: INVENTORY_SCHEMA_VERSION,
            ready: Vec::new(),
        });
    }

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read model inventory {}", path.display()))?;
    let inventory: ModelInventory = toml::from_str(&text)
        .with_context(|| format!("failed to parse model inventory {}", path.display()))?;
    if inventory.schema_version != INVENTORY_SCHEMA_VERSION {
        return Ok(ModelInventory {
            schema_version: INVENTORY_SCHEMA_VERSION,
            ready: Vec::new(),
        });
    }
    Ok(inventory)
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

fn inventory_entry_matches_catalog(
    entry: &ModelInventoryEntry,
    catalog_entry: ModelCatalogEntry,
) -> bool {
    entry.filename == catalog_entry.filename
        && entry.size_bytes == catalog_entry.size_bytes
        && entry.sha1 == catalog_entry.sha1
}

fn cached_model_file_exists(root: &Path, entry: ModelCatalogEntry) -> bool {
    fs::metadata(model_path(root, entry)).is_ok_and(|metadata| metadata.len() == entry.size_bytes)
}

impl From<ModelCatalogEntry> for ModelInventoryEntry {
    fn from(entry: ModelCatalogEntry) -> Self {
        Self {
            id: entry.id.to_string(),
            filename: entry.filename.to_string(),
            size_bytes: entry.size_bytes,
            sha1: entry.sha1.to_string(),
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
    fn inventory_cache_marks_model_ready_without_hashing_file_contents() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        create_sparse_model(&root, entry);

        let ready = mark_model_ready(&root, entry).unwrap();

        assert!(ready.contains(entry.id));
        assert!(load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_cached_file_is_not_ready() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();

        mark_model_ready(&root, entry).unwrap();

        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn wrong_size_cached_file_is_not_ready() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        fs::create_dir_all(&root).unwrap();
        File::create(model_path(&root, entry))
            .unwrap()
            .set_len(1)
            .unwrap();

        mark_model_ready(&root, entry).unwrap();

        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_model_updates_inventory_cache() {
        let root = temp_model_root();
        let entry = model_catalog_entry("tiny").unwrap();
        create_sparse_model(&root, entry);
        mark_model_ready(&root, entry).unwrap();

        let ready = remove_model_from_inventory(&root, entry.id).unwrap();

        assert!(!ready.contains(entry.id));
        assert!(!load_ready_model_ids(&root).contains(entry.id));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_model_root() -> PathBuf {
        let id = TEST_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("myapp-model-inventory-test-{id}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn create_sparse_model(root: &Path, entry: ModelCatalogEntry) {
        fs::create_dir_all(root).unwrap();
        File::create(model_path(root, entry))
            .unwrap()
            .set_len(entry.size_bytes)
            .unwrap();
    }
}
