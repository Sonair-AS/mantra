use std::sync::atomic::Ordering;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cfg = mantra::cfg::Config::parse();

    mantra::logger::init(cfg.warnings_as_errors);

    if let Err(err) = mantra::run(cfg).await {
        println!("{err}");
        std::process::exit(-1);
    }

    if mantra::logger::HAD_WARNINGS.load(Ordering::Relaxed) {
        std::process::exit(-1);
    }
}
