use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};
use shared::{MODEL_CATALOG, ModelCatalogEntry, model_catalog_entry};

pub fn scan_ready_model_ids(root: &Path) -> HashSet<String> {
    MODEL_CATALOG
        .iter()
        .filter_map(|entry| {
            let entry = model_catalog_entry(entry.id)?;
            is_model_ready(root, entry).then(|| entry.id.to_string())
        })
        .collect()
}

pub fn is_model_ready(root: &Path, entry: ModelCatalogEntry) -> bool {
    let path = model_path(root, entry);
    path.exists() && sha1_file(&path).is_ok_and(|sha1| sha1 == entry.sha1)
}

pub fn model_path(root: &Path, entry: ModelCatalogEntry) -> PathBuf {
    root.join(entry.filename)
}

pub fn partial_model_path(root: &Path, entry: ModelCatalogEntry) -> PathBuf {
    model_path(root, entry).with_extension("bin.part")
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let bytes = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}
