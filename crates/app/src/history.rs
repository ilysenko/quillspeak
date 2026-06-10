use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    Transcription,
    Script,
}

impl HistorySource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transcription => "Transcription",
            Self::Script => "Script",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub created_at_unix_ms: u128,
    pub recording_id: u64,
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub model_id: String,
    pub language: String,
    pub source: HistorySource,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("failed to resolve user data directory")?;
        Ok(Self {
            path: base_dirs.data_dir().join("myapp/history.jsonl"),
        })
    }

    #[cfg(test)]
    fn for_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<HistoryEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)
            .with_context(|| format!("failed to open history {}", self.path.display()))?;
        let mut entries = Vec::new();
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.with_context(|| {
                format!(
                    "failed to read history line {} from {}",
                    line_index + 1,
                    self.path.display()
                )
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<HistoryEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(error) => warn!(
                    ?error,
                    history_path = %self.path.display(),
                    line = line_index + 1,
                    "skipping malformed history entry"
                ),
            }
        }
        Ok(entries)
    }

    pub fn append(&self, entry: &HistoryEntry) -> Result<()> {
        if entry.text.trim().is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create history dir {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open history {}", self.path.display()))?;
        serde_json::to_writer(&mut file, entry).context("failed to encode history entry")?;
        file.write_all(b"\n")
            .with_context(|| format!("failed to write history {}", self.path.display()))?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("failed to clear history {}", self.path.display())),
        }
    }
}

pub fn unix_time_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn history_store_appends_loads_and_clears_jsonl() {
        let root = temp_root();
        let store = HistoryStore::for_path(root.join("history.jsonl"));
        let first = test_entry(1, "hello", HistorySource::Transcription);
        let second = test_entry(2, "translated", HistorySource::Script);

        store.append(&first).expect("first append should succeed");
        store.append(&second).expect("second append should succeed");

        assert_eq!(
            store.load().expect("history should load"),
            vec![first, second]
        );

        store.clear().expect("history clear should succeed");
        assert_eq!(store.load().expect("cleared history should load"), vec![]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn history_store_skips_empty_text_appends() {
        let root = temp_root();
        let store = HistoryStore::for_path(root.join("history.jsonl"));

        store
            .append(&test_entry(1, "   ", HistorySource::Transcription))
            .expect("empty append should be ignored");

        assert!(!store.path().exists());
        let _ = fs::remove_dir_all(root);
    }

    fn test_entry(recording_id: u64, text: &str, source: HistorySource) -> HistoryEntry {
        HistoryEntry {
            created_at_unix_ms: u128::from(recording_id),
            recording_id,
            shortcut_id: "default".to_string(),
            shortcut_name: "Default".to_string(),
            model_id: "tiny".to_string(),
            language: "auto".to_string(),
            source,
            text: text.to_string(),
        }
    }

    fn temp_root() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("myapp-history-test-{id}"));
        let _ = fs::remove_dir_all(&root);
        root
    }
}
