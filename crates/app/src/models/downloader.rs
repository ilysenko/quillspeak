use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use shared::model_catalog_entry;
use tracing::info;

use crate::command::AppCommand;
use crate::models::inventory::{model_path, partial_model_path, sha1_file};

pub fn start_download(root: &Path, model_id: String, command_tx: mpsc::Sender<AppCommand>) {
    let root = root.to_path_buf();
    thread::spawn(move || {
        let result =
            download_model(&root, &model_id, &command_tx).map_err(|error| format!("{error:#}"));
        let _ = command_tx.send(AppCommand::ModelDownloadFinished { model_id, result });
    });
}

fn download_model(
    root: &Path,
    model_id: &str,
    command_tx: &mpsc::Sender<AppCommand>,
) -> Result<()> {
    let entry = model_catalog_entry(model_id).ok_or_else(|| anyhow!("unknown model {model_id}"))?;
    let partial = partial_model_path(root, entry);
    let result = download_model_inner(root, model_id, command_tx);
    if result.is_err() && partial.exists() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn download_model_inner(
    root: &Path,
    model_id: &str,
    command_tx: &mpsc::Sender<AppCommand>,
) -> Result<()> {
    let entry = model_catalog_entry(model_id).ok_or_else(|| anyhow!("unknown model {model_id}"))?;
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create model directory {}", root.display()))?;
    let target = model_path(root, entry);
    let partial = partial_model_path(root, entry);

    info!(model_id, url = entry.url, "downloading whisper.cpp model");
    let client = Client::builder()
        .build()
        .context("failed to build HTTP client")?;
    let mut response = client
        .get(entry.url)
        .send()
        .context("failed to start model download")?
        .error_for_status()
        .context("model download returned HTTP error")?;
    let total = response.content_length().or(Some(entry.size_bytes));
    let mut file = File::create(&partial)
        .with_context(|| format!("failed to create partial model {}", partial.display()))?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let bytes = response
            .read(&mut buffer)
            .context("failed to read model download stream")?;
        if bytes == 0 {
            break;
        }
        file.write_all(&buffer[..bytes])
            .context("failed to write model download chunk")?;
        downloaded += bytes as u64;
        let _ = command_tx.send(AppCommand::ModelDownloadProgress {
            model_id: model_id.to_string(),
            downloaded,
            total,
        });
    }
    file.flush().context("failed to flush partial model")?;

    let sha1 = sha1_file(&partial)?;
    if sha1 != entry.sha1 {
        return Err(anyhow!(
            "downloaded model hash mismatch: expected {}, got {}",
            entry.sha1,
            sha1
        ));
    }

    fs::rename(&partial, &target).with_context(|| {
        format!(
            "failed to move downloaded model {} to {}",
            partial.display(),
            target.display()
        )
    })?;
    info!(model_id, path = %target.display(), "model download finished");
    Ok(())
}
