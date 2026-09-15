//! GitHub release notifications. Checks never change the installed binary.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Context, bail};
use semver::Version;
use serde::Deserialize;

use super::{EditorMode, EditorState};
use crate::storage;

const RELEASE_API: &str = "https://api.github.com/repos/JackDerksen/redox/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/JackDerksen/redox/releases/latest";
const CACHE_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub(super) struct UpdateCheck {
    receiver: Receiver<Result<Version, String>>,
    manual: bool,
}

impl EditorState {
    pub(crate) fn configure_update_checks(&mut self, enabled: bool) {
        if enabled {
            self.start_update_check(false);
        } else if self
            .update_check
            .as_ref()
            .is_some_and(|check| !check.manual)
        {
            self.update_check = None;
        }
    }

    pub(crate) fn start_update_check(&mut self, manual: bool) {
        if manual {
            self.set_status("checking for Redox updates...");
        }
        if self
            .update_check
            .as_ref()
            .is_some_and(|check| !manual || check.manual)
        {
            return;
        }
        // A manual check bypasses the startup cache, even while it is being read.
        let cache_path = storage::state_root().join("update-check.json");
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("redox-update-check".into())
            .spawn(move || {
                let result = latest_release(&cache_path, manual).map_err(|error| error.to_string());
                let _ = sender.send(result);
            }) {
            Ok(_) => self.update_check = Some(UpdateCheck { receiver, manual }),
            Err(error) if manual => self.set_status(format!("update check failed: {error}")),
            Err(_) => {}
        }
    }

    pub(super) fn update_check_can_notify(&self) -> bool {
        // Deferred notices need no polling until input or a status expiry wakes the editor.
        self.update_check.as_ref().is_some_and(|check| {
            self.mode == EditorMode::Normal
                && self.recording_macro_register().is_none()
                && (check.manual || self.status_msg.is_none())
        })
    }

    pub(super) fn poll_update_check(&mut self) {
        if !self.update_check_can_notify() {
            return;
        }
        let Some(check) = &self.update_check else {
            return;
        };
        let result = match check.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("update worker stopped".into()),
        };
        let manual = check.manual;
        self.update_check = None;
        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).expect("valid Cargo package version");
        match result {
            Ok(latest) => {
                if let Some(message) = update_message(&latest, &current, manual) {
                    self.set_status(message);
                }
            }
            Err(error) if manual => self.set_status(format!("update check failed: {error}")),
            Err(_) => {} // Offline startup stays quiet; :check-update reports failures.
        }
    }
}

fn update_message(latest: &Version, current: &Version, manual: bool) -> Option<String> {
    if latest.cmp_precedence(current).is_gt() {
        Some(format!(
            "Redox {latest} is available (running {current})\n\
             Cargo installs: cargo install redox-editor --locked\n\
             Release binaries: {RELEASE_PAGE}"
        ))
    } else {
        manual.then(|| format!("Redox {current} is up to date (latest release: {latest})"))
    }
}

fn latest_release(cache_path: &Path, force: bool) -> anyhow::Result<Version> {
    if !force
        && fs::metadata(cache_path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .is_ok_and(|age| age < CACHE_LIFETIME)
        && let Ok(bytes) = fs::read(cache_path)
        && let Ok(version) = release_version(&bytes)
    {
        return Ok(version);
    }

    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "3",
            "--max-time",
            "5",
            "--max-filesize",
            "1048576",
            "--header",
            "Accept: application/vnd.github+json",
            "--user-agent",
            concat!("redox/", env!("CARGO_PKG_VERSION")),
            RELEASE_API,
        ])
        .stdin(Stdio::null())
        .output()
        .context("could not run curl; install curl to check for updates")?;
    if !output.status.success() {
        bail!("GitHub request failed ({})", output.status);
    }
    let version = release_version(&output.stdout)?;
    // This cache is optional; an unwritable state directory must not hide an update.
    if let Some(parent) = cache_path.parent()
        && fs::create_dir_all(parent).is_ok()
    {
        let _ = fs::write(cache_path, &output.stdout);
    }
    Ok(version)
}

fn release_version(bytes: &[u8]) -> anyhow::Result<Version> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        draft: bool,
        prerelease: bool,
    }
    let release: Release =
        serde_json::from_slice(bytes).context("invalid GitHub release response")?;
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("invalid release version")?;
    if release.draft || release.prerelease || !version.pre.is_empty() {
        bail!("GitHub did not return a stable release");
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_notices_use_stable_semantic_versions() {
        for (tag, valid) in [
            ("v0.10.0", true),
            ("0.9.1", true),
            ("v1.0.0-beta.1", false),
            ("v1.0.0\u{1b}[31m", false),
        ] {
            let response =
                serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false});
            assert_eq!(
                release_version(response.to_string().as_bytes()).is_ok(),
                valid
            );
        }
        for field in ["draft", "prerelease"] {
            let mut response =
                serde_json::json!({"tag_name": "v1.0.0", "draft": false, "prerelease": false});
            response[field] = true.into();
            assert!(release_version(response.to_string().as_bytes()).is_err());
        }
        for (latest, current, available) in [
            ("0.10.0", "0.9.0", true),
            ("0.9.0", "0.10.0", false),
            ("0.9.0", "0.9.0", false),
            ("1.0.0", "1.0.0-beta.1", true),
            ("1.0.0+release", "1.0.0+local", false),
        ] {
            let latest = Version::parse(latest).unwrap();
            let current = Version::parse(current).unwrap();
            assert_eq!(
                update_message(&latest, &current, false).is_some(),
                available
            );
            assert!(update_message(&latest, &current, true).is_some());
        }
    }

    #[test]
    fn startup_uses_cached_release_and_defers_notice_until_status_clears() {
        let directory = tempfile::tempdir().unwrap();
        let cache_path = directory.path().join("update-check.json");
        fs::write(
            &cache_path,
            br#"{"tag_name":"v999.0.0","draft":false,"prerelease":false}"#,
        )
        .unwrap();
        let latest = latest_release(&cache_path, false).unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok(latest)).unwrap();
        let mut state =
            EditorState::new(redox_core::EditorSession::open_initial_unnamed().unwrap());
        state.update_check = Some(UpdateCheck {
            receiver,
            manual: false,
        });
        state.set_status("file changed on disk");
        state.poll_update_check();
        assert_eq!(state.status_msg.as_deref(), Some("file changed on disk"));
        assert!(state.update_check.is_some());
        state.clear_status();
        state.poll_update_check();
        assert!(
            state
                .status_msg
                .as_deref()
                .unwrap()
                .contains("Redox 999.0.0 is available")
        );
        assert!(state.update_check.is_none());

        for manual in [false, true] {
            state.clear_status();
            let (sender, receiver) = mpsc::channel();
            sender.send(Err("offline".into())).unwrap();
            state.update_check = Some(UpdateCheck { receiver, manual });
            state.poll_update_check();
            assert_eq!(state.status_msg.is_some(), manual);
            assert!(state.update_check.is_none());
        }
    }
}
