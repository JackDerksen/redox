use std::process::Command;

#[test]
fn help_and_version_work_without_a_terminal_or_valid_config() {
    for (argument, expected) in [
        ("--version", concat!("redox ", env!("CARGO_PKG_VERSION"))),
        ("-V", concat!("redox ", env!("CARGO_PKG_VERSION"))),
        ("--help", "Usage: redox"),
        ("-h", "Usage: redox"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_redox"))
            .env("REDOX_CONFIG", "/nonexistent/redox-config.toml")
            .arg(argument)
            .output()
            .unwrap();
        assert!(output.status.success(), "{argument}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        assert!(output.stderr.is_empty());
    }
}
