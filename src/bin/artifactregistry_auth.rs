//! Compatibility executable for the official helper's documented CLI arguments.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    node_auth::cli::execute().await
}
