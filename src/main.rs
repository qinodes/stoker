fn main() {
    if let Err(error) = stoker::cli::run() {
        let message = stoker::output::escape_control_characters(&format!("error: {error:#}"));
        eprintln!(
            "{}",
            stoker::output::paint(
                message,
                crossterm::style::Color::Red,
                stoker::output::stderr_color_enabled(),
            )
        );
        std::process::exit(1);
    }
}
