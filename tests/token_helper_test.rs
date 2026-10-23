//! Token-helper process contracts, including builds without CLI dependencies.

use std::{fs, process::Command};
use tempfile::tempdir;

fn helper() -> std::path::PathBuf {
  env!("CARGO_BIN_EXE_node-auth-token").into()
}

#[test]
fn token_binary_prints_only_token_and_never_writes_files() {
  let dir = tempdir().unwrap();

  fs::write(dir.path().join(".npmrc"), "# untouched\n").unwrap();

  let output = Command::new(helper())
    .current_dir(dir.path())
    .env("NODE_AUTH_TOKEN", "fixture-token")
    .output()
    .expect("node-auth-token must be built");
  assert!(output.status.success(), "{:?}", output);
  assert_eq!(output.stdout, b"fixture-token\n");
  assert!(output.stderr.is_empty());
  assert_eq!(
    fs::read_to_string(dir.path().join(".npmrc")).unwrap(),
    "# untouched\n"
  );
  assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn token_binary_failure_has_empty_stdout_and_nonzero_exit() {
  let output = Command::new(helper())
    .env("NODE_AUTH_TOKEN", "secret\ninvalid")
    .output()
    .unwrap();
  assert!(!output.status.success());
  assert!(output.stdout.is_empty());
  assert!(!output.stderr.is_empty());
  assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
}

#[test]
fn token_helper_rejects_arguments_without_output() {
  let out = Command::new(helper())
    .arg("unexpected")
    .env("NODE_AUTH_TOKEN", "fixture")
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(out.stdout.is_empty());
}

#[test]
fn token_helper_preserves_gcloud_fallback_and_auth_failure() {
  let dir = tempdir().unwrap();

  #[cfg(unix)]
  let gcloud = dir.path().join("gcloud");

  #[cfg(windows)]
  let gcloud = dir.path().join("gcloud.cmd");

  for success in [true, false] {
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;

      fs::write(
        &gcloud,
        if success {
          "#!/bin/sh\nprintf 'fallback-token\\n'\n"
        } else {
          "#!/bin/sh\necho secret-provider-diagnostic >&2\nexit 1\n"
        },
      )
      .unwrap();
      fs::set_permissions(&gcloud, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(windows)]
    fs::write(
      &gcloud,
      if success {
        "@echo off\r\necho fallback-token\r\n"
      } else {
        "@echo off\r\necho secret-provider-diagnostic 1>&2\r\nexit /b 1\r\n"
      },
    )
    .unwrap();

    // An explicitly missing ADC file fails before metadata requests or real user ADC.
    let out = Command::new(helper())
      .env_remove("NODE_AUTH_TOKEN")
      .env(
        "GOOGLE_APPLICATION_CREDENTIALS",
        dir.path().join("missing.json"),
      )
      .env("PATH", dir.path())
      .current_dir(dir.path())
      .output()
      .unwrap();
    assert_eq!(out.status.success(), success);

    if success {
      assert_eq!(out.stdout, b"fallback-token\n");
      assert!(out.stderr.is_empty());
    } else {
      assert!(out.stdout.is_empty());
      let stderr = String::from_utf8_lossy(&out.stderr);
      assert!(stderr.contains("ADC or gcloud"));
      assert!(!stderr.contains("secret-provider-diagnostic"));
    }
  }
}
