//! Command-line arguments and the launch target they select.

use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

use minui::prelude::render::Color;

pub(crate) enum LaunchTarget {
    Empty,
    File(PathBuf),
    Explorer(PathBuf),
}

pub(crate) struct LaunchOptions {
    pub(crate) target: LaunchTarget,
    pub(crate) config: LaunchConfig,
}

#[derive(Default)]
pub(crate) struct LaunchConfig {
    pub(crate) path: Option<PathBuf>,
    pub(crate) default_only: bool,
}

impl LaunchConfig {
    pub(crate) fn load(&self) -> anyhow::Result<(crate::config::Config, Option<PathBuf>)> {
        if self.default_only {
            Ok((crate::config::Config::default(), None))
        } else {
            crate::config::Config::load(self.path.as_deref())
        }
    }
}

pub(crate) fn print_cli_logo(
    wordmark: &str,
    color_enabled: bool,
    style: crate::ui::style::AboutStyle,
) {
    for (line, color) in [
        ("┏━┓", style.logo_red),
        (wordmark, style.logo_white),
        ("  ┗━┛", style.logo_blue),
    ] {
        if color_enabled {
            let text_style = minui::Style::from(color.with_colors(color.fg, Color::Transparent));
            println!("{}\x1b[0m", text_style.to_crossterm().apply(line));
        } else {
            println!("{line}");
        }
    }
}

pub(crate) fn parse_launch_options() -> anyhow::Result<Option<LaunchOptions>> {
    let stdout_is_terminal = std::io::stdout().is_terminal();
    let color_enabled = stdout_is_terminal
        && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && env::var("TERM").as_deref() != Ok("dumb");
    let mut args = env::args().skip(1);
    let mut config = LaunchConfig::default();
    let mut target_path = None;
    let mut parse_options = true;
    let mut show_help = None;
    while let Some(raw) = args.next() {
        if parse_options && raw == "--" {
            parse_options = false;
        } else if parse_options && matches!(raw.as_str(), "--version" | "-V") {
            show_help.get_or_insert(false);
        } else if parse_options && matches!(raw.as_str(), "--help" | "-h") {
            show_help.get_or_insert(true);
        } else if parse_options && raw == "--default-config" {
            config.default_only = true;
        } else if parse_options && raw == "--config" {
            config.path =
                Some(PathBuf::from(args.next().ok_or_else(|| {
                    anyhow::anyhow!("--config requires a path")
                })?));
        } else if parse_options && let Some(path) = raw.strip_prefix("--config=") {
            config.path = Some(PathBuf::from(path));
        } else if parse_options && raw.starts_with('-') {
            anyhow::bail!("unknown option: {raw}");
        } else if target_path.replace(PathBuf::from(&raw)).is_some() {
            anyhow::bail!("only one file or directory may be opened at launch");
        }
    }
    if let Some(show_help) = show_help {
        // Help remains available when the user's configuration is invalid.
        let style = config
            .load()
            .and_then(|(config, _)| config.style())
            .unwrap_or_default();
        if show_help {
            print_cli_logo("Redox - A tasteful text editor", color_enabled, style.about);
            let (heading, bold, reset) = if color_enabled && style.text_formatting {
                ("\x1b[1;4m", "\x1b[1m", "\x1b[0m")
            } else {
                ("", "", "")
            };
            println!(
                "\n{heading}Usage:{reset} {bold}redox{reset} [OPTIONS] [FILE_OR_DIRECTORY]\n\n\
                 {heading}Options:{reset}\n  \
                 {bold}--config{reset} <PATH>   Use a configuration file\n  \
                 {bold}--default-config{reset}  Use only built-in defaults, ignoring configuration files\n  \
                 {bold}-h, --help{reset}        Print help\n  \
                 {bold}-V, --version{reset}     Print version\n  \
                 {bold}--{reset}                Treat remaining arguments as paths"
            );
            return Ok(None);
        } else {
            if stdout_is_terminal {
                print_cli_logo(
                    concat!("Redox - v", env!("CARGO_PKG_VERSION")),
                    color_enabled,
                    style.about,
                );
            } else {
                println!("redox {}", env!("CARGO_PKG_VERSION"));
            }
            return Ok(None);
        }
    }
    let target = match target_path {
        None => LaunchTarget::Empty,
        Some(path) if path.is_dir() => LaunchTarget::Explorer(path),
        Some(path) => LaunchTarget::File(path),
    };
    Ok(Some(LaunchOptions { target, config }))
}
