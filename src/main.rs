//! Main CLI entrypoint for `node-auth`.

#[tokio::main]
async fn main() -> std::process::ExitCode {
  node_auth::cli::execute().await
}
