fn main() {
    if let Err(error) = stoker::cli::run() {
        eprintln!(
            "{}",
            stoker::output::paint(
                format!("error: {error:#}"),
                crossterm::style::Color::Red,
                stoker::output::stderr_color_enabled(),
            )
        );
        std::process::exit(1);
    }
}
