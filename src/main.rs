//! Main CLI entrypoint for `node-auth`.

use clap::Parser;
use colored::Colorize;
use node_auth::cli::Cli;
use node_auth::{logger, run};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    logger::init(cli.verbose, !cli.print_token);

    let options = cli.to_options();

    if let Err(err) = run(&options).await {
        log::error!("{err}");
        eprintln!("{} {err}", "Error:".red().bold());
        std::process::exit(1);
    }
}