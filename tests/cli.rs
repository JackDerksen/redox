use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

#[test]
fn help_and_version_work_without_a_terminal_or_valid_config() {
    // A file name that is not UTF-8 is still a valid path to open.
    let latin1_path = OsStr::from_bytes(b"caf\xe9.txt");
    for (arguments, expected) in [
        (
            &[OsStr::new("--version")][..],
            concat!("redox ", env!("CARGO_PKG_VERSION")),
        ),
        (
            &[OsStr::new("-V")],
            concat!("redox ", env!("CARGO_PKG_VERSION")),
        ),
        (&[OsStr::new("--help")], "Usage: redox"),
        (&[OsStr::new("-h")], "Usage: redox"),
        (
            &[OsStr::new("--version"), latin1_path],
            concat!("redox ", env!("CARGO_PKG_VERSION")),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_redox"))
            .env("REDOX_CONFIG", "/nonexistent/redox-config.toml")
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{arguments:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        assert!(output.stderr.is_empty());
    }
}
