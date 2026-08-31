fn main() {
    if let Err(error) = stoker::cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
