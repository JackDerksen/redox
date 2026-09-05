use std::io;
use std::path::{Path, PathBuf};

use url::Url;

use crate::provider::{LanguageServerProvider, ProviderId};

/// Converts a local path to a percent-encoded `file:` URI.
pub fn file_uri(path: &Path) -> io::Result<String> {
    Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path is not a valid file URI"))
}

/// Converts a `file:` URI to a local path.
#[must_use]
pub fn file_path_from_uri(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

/// Finds the workspace root selected by a provider adapter.
#[must_use]
pub fn workspace_root_for(
    path: &Path,
    provider: &impl LanguageServerProvider,
    launch_dir: &Path,
) -> PathBuf {
    provider.workspace_root(path, launch_dir)
}

/// Finds a conservative workspace root for custom providers.
#[must_use]
pub fn default_workspace_root(path: &Path, launch_dir: &Path) -> PathBuf {
    let Some(start_dir) = path.parent() else {
        return launch_dir.to_path_buf();
    };
    find_nearest_ancestor_with_any_marker(start_dir, &[".git"])
        .unwrap_or_else(|| start_dir.to_path_buf())
}

pub(crate) fn built_in_workspace_root_for(
    path: &Path,
    provider_id: ProviderId,
    launch_dir: &Path,
) -> PathBuf {
    let Some(start_dir) = path.parent() else {
        return launch_dir.to_path_buf();
    };

    match provider_id {
        ProviderId::RustAnalyzer => {
            find_outermost_ancestor_with_any_marker(start_dir, &["Cargo.toml", "rust-project.json"])
                .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"]))
        }
        ProviderId::Gopls => find_outermost_ancestor_with_any_marker(start_dir, &["go.work"])
            .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &["go.mod"]))
            .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::TypeScriptLanguageServer => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[
                "tsconfig.json",
                "jsconfig.json",
                "package.json",
                "deno.json",
                "deno.jsonc",
            ],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::Pyright => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[
                "pyproject.toml",
                "setup.py",
                "setup.cfg",
                "requirements.txt",
            ],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::Clangd => find_nearest_ancestor_with_any_marker(
            start_dir,
            &["compile_commands.json", "compile_flags.txt", ".clangd"],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::LuaLanguageServer => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[".luarc.json", ".luarc.jsonc", "stylua.toml", ".git"],
        ),
        ProviderId::Taplo => find_nearest_ancestor_with_any_marker(
            start_dir,
            &["taplo.toml", ".taplo.toml", "Cargo.toml", ".git"],
        ),
        ProviderId::Marksman => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[".marksman.toml", "package.json", ".git"],
        ),
        ProviderId::YamlLanguageServer
        | ProviderId::JsonLanguageServer
        | ProviderId::HtmlLanguageServer
        | ProviderId::CssLanguageServer => {
            find_nearest_ancestor_with_any_marker(start_dir, &["package.json", ".git"])
        }
    }
    .unwrap_or_else(|| start_dir.to_path_buf())
}

fn find_nearest_ancestor_with_any_marker(start_dir: &Path, markers: &[&str]) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .find(|dir| markers.iter().any(|marker| dir.join(marker).exists()))
        .map(Path::to_path_buf)
}

fn find_outermost_ancestor_with_any_marker(start_dir: &Path, markers: &[&str]) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .filter(|dir| markers.iter().any(|marker| dir.join(marker).exists()))
        .last()
        .map(Path::to_path_buf)
}
