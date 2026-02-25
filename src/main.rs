fn main() {
    if let Err(err) = redox_tui::run() {
        eprintln!("redox error: {err}");
        std::process::exit(1);
    }
}
