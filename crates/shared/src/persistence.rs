use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write_text(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary_path = temporary_path_for(path);
    let result = write_and_rename(path, &temporary_path, contents);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn write_and_rename(path: &Path, temporary_path: &Path, contents: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);

    fs::rename(temporary_path, path)?;
    if let Some(parent) = path.parent()
        && let Ok(directory) = fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn temporary_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::{env, fs};

    use super::*;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn atomic_write_text_creates_parent_and_replaces_contents() {
        let root = temp_root();
        let path = root.join("nested/config.toml");

        atomic_write_text(&path, "first").expect("first write should succeed");
        atomic_write_text(&path, "second").expect("second write should succeed");

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert!(fs::read_dir(path.parent().unwrap()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("quillspeak-atomic-write-test-{id}"));
        let _ = fs::remove_dir_all(&root);
        root
    }
}
