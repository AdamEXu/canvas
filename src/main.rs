mod cli;
mod format;
mod model;
mod render;
mod server;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = cli::run(args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
