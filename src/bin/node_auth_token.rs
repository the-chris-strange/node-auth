//! Argument-free token helper, also available without the `cli` feature.
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
  let result = (|| {
    if std::env::args_os().len() != 1 {
      return Err(node_auth::AuthError::Config(
        "node-auth-token takes no arguments".into(),
      ));
    }
    let options = node_auth::Options {
      print_token: true,
      token: std::env::var("NODE_AUTH_TOKEN").ok(),
      ..Default::default()
    };
    let node_auth::RunOutcome::Token(token) = node_auth::run_blocking(&options)? else {
      unreachable!("print_token always returns a token")
    };
    writeln!(std::io::stdout().lock(), "{token}")?;
    Ok::<_, node_auth::AuthError>(())
  })();
  match result {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => {
      eprintln!("{error}");
      eprintln!("Check Google ADC (gcloud auth application-default login) or your gcloud session.");
      ExitCode::FAILURE
    }
  }
}
