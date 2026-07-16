//! Filesystem locations and migration for editor-managed state.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "redox";

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
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            let name = destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("state");
            let temporary = destination.with_file_name(format!(".{name}.migration-tmp"));
            if temporary.is_dir() {
                fs::remove_dir_all(&temporary)?;
            } else if temporary.exists() {
                fs::remove_file(&temporary)?;
            }
            if source.is_dir() {
                copy_directory(source, &temporary)?;
            } else {
                fs::copy(source, &temporary)?;
            }
            fs::rename(&temporary, destination)?;
            if source.is_dir() {
                fs::remove_dir_all(source)
            } else {
                fs::remove_file(source)
            }
        }
    }
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
