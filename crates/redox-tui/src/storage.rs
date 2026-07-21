//! Filesystem locations and migration for editor-managed state.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use redox_core::{TextBuffer, UndoHistory};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "redox";
const UNDO_HISTORY_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct UndoHistorySnapshot {
    version: u32,
    path: String,
    content_hash: u64,
    content_len_chars: usize,
    history: UndoHistory,
}

pub fn state_root() -> PathBuf {
    if let Some(root) = xdg_root("XDG_STATE_HOME") {
        return root.join(APP_DIR);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".local/state").join(APP_DIR);
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".local/state")
        .join(APP_DIR)
}

pub fn config_root() -> PathBuf {
    if let Some(root) = xdg_root("XDG_CONFIG_HOME") {
        return root.join(APP_DIR);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join(APP_DIR);
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
        .join(APP_DIR)
}

fn xdg_root(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

pub fn pinned_files_path() -> PathBuf {
    let destination = state_root().join("pinned-files.json");
    let legacy = config_root().join("pinned_files.txt");
    let _ = migrate_path(&legacy, &destination);
    if destination.exists() || !legacy.exists() {
        destination
    } else {
        legacy
    }
}

pub fn installed_tools_path() -> PathBuf {
    let destination = state_root().join("installed-tools.json");
    let legacy = config_root().join("installed_lsps.json");
    let _ = migrate_path(&legacy, &destination);
    if destination.exists() || !legacy.exists() {
        destination
    } else {
        legacy
    }
}

pub fn undo_history_root() -> PathBuf {
    #[cfg(test)]
    {
        return env::temp_dir().join(format!("redox-test-undo-history-{}", std::process::id()));
    }

    #[cfg(not(test))]
    state_root().join("undo-history")
}

pub fn load_undo_history(
    path: &Path,
    buffer: &TextBuffer,
    max_records: usize,
) -> io::Result<Option<UndoHistory>> {
    let snapshot_path = undo_history_path(path);
    let bytes = match fs::read(snapshot_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let snapshot: UndoHistorySnapshot = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let content = buffer.to_string();
    if snapshot.version != UNDO_HISTORY_VERSION
        || snapshot.path != path.to_string_lossy()
        || snapshot.content_len_chars != buffer.len_chars()
        || snapshot.content_hash != stable_hash(content.as_bytes())
    {
        return Ok(None);
    }

    let mut history = snapshot.history;
    if !history.prepare_after_load(max_records) {
        return Ok(None);
    }
    Ok(Some(history))
}

pub fn save_undo_history(
    path: &Path,
    buffer: &TextBuffer,
    history: &UndoHistory,
) -> io::Result<()> {
    if history.tree_entries().len() <= 1 {
        return remove_undo_history(path);
    }

    let root = undo_history_root();
    fs::create_dir_all(&root)?;
    let content = buffer.to_string();
    let snapshot = UndoHistorySnapshot {
        version: UNDO_HISTORY_VERSION,
        path: path.to_string_lossy().into_owned(),
        content_hash: stable_hash(content.as_bytes()),
        content_len_chars: buffer.len_chars(),
        history: history.clone(),
    };
    let destination = undo_history_path(path);
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(&mut temporary, &snapshot).map_err(io::Error::other)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

pub fn remove_undo_history(path: &Path) -> io::Result<()> {
    match fs::remove_file(undo_history_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn undo_history_path(path: &Path) -> PathBuf {
    undo_history_root().join(format!(
        "{:016x}.json",
        stable_hash(path.to_string_lossy().as_bytes())
    ))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Move every known legacy state entry out of the user configuration directory.
pub fn migrate_legacy_state() -> io::Result<()> {
    fs::create_dir_all(state_root())?;
    migrate_path(
        &config_root().join("pinned_files.txt"),
        &state_root().join("pinned-files.json"),
    )?;
    migrate_path(
        &config_root().join("installed_lsps.json"),
        &state_root().join("installed-tools.json"),
    )?;
    migrate_path(
        &config_root().join("undo-tree"),
        &state_root().join("undo-history"),
    )?;
    migrate_path(
        &config_root().join("lsp.json"),
        &state_root().join("legacy/lsp.json"),
    )
}

fn migrate_path(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.exists() || destination.exists() {
        return Ok(());
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let migration_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(parent.join(".redox-migration.lock"))?;
    migration_lock.lock()?;

    if !source.exists() || destination.exists() {
        return Ok(());
    }
    match rename_noreplace(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => {
            let name = destination
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| "state".into());
            let prefix = format!(".{name}.migration-{}-", std::process::id());
            let staging = tempfile::Builder::new()
                .prefix(&prefix)
                .tempdir_in(parent)?;
            let temporary = staging.path().join("data");
            let source_is_dir = source.is_dir();
            if source_is_dir {
                copy_directory(source, &temporary)?;
            } else {
                fs::copy(source, &temporary)?;
            }
            match rename_noreplace(&temporary, destination) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
                Err(error) => return Err(error),
            }
            if source_is_dir {
                fs::remove_dir_all(source)
            } else {
                fs::remove_file(source)
            }
        }
    }
}

#[cfg(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "redox"
))]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(target_os = "windows")]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    target_os = "redox",
    target_os = "windows"
)))]
fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this platform",
    ))
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    // I changed the naming convention for these, so just in case...
    #[test]
    fn legacy_entries_move_to_kebab_case_state_paths() {
        let _lock = crate::app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("redox_storage_test_{nonce}"));
        let config = root.join("config");
        let state = root.join("state");
        let previous_config = env::var_os("XDG_CONFIG_HOME");
        let previous_state = env::var_os("XDG_STATE_HOME");
        unsafe {
            env::set_var("XDG_CONFIG_HOME", &config);
            env::set_var("XDG_STATE_HOME", &state);
        }
        let legacy_root = config.join(APP_DIR);
        fs::create_dir_all(legacy_root.join("undo-tree")).unwrap();
        fs::write(legacy_root.join("pinned_files.txt"), "[]").unwrap();
        fs::write(legacy_root.join("installed_lsps.json"), "[]").unwrap();
        fs::write(legacy_root.join("lsp.json"), "{}").unwrap();
        fs::write(legacy_root.join("undo-tree/history.rut"), "history").unwrap();

        migrate_legacy_state().unwrap();

        let migrated = state.join(APP_DIR);
        assert!(migrated.join("pinned-files.json").is_file());
        assert!(migrated.join("installed-tools.json").is_file());
        assert!(migrated.join("legacy/lsp.json").is_file());
        assert!(migrated.join("undo-history/history.rut").is_file());
        assert!(!legacy_root.join("pinned_files.txt").exists());
        assert!(!legacy_root.join("installed_lsps.json").exists());
        assert!(!legacy_root.join("lsp.json").exists());
        assert!(!legacy_root.join("undo-tree").exists());

        unsafe {
            match previous_config {
                Some(value) => env::set_var("XDG_CONFIG_HOME", value),
                None => env::remove_var("XDG_CONFIG_HOME"),
            }
            match previous_state {
                Some(value) => env::set_var("XDG_STATE_HOME", value),
                None => env::remove_var("XDG_STATE_HOME"),
            }
        }
        let _ = fs::remove_dir_all(root);
    }
}
