//! Isolated process tests: no live Google credentials or user configuration.

#![cfg(feature = "cli")]
use std::fs;
use std::process::Command;
use tempfile::{TempDir, tempdir};

fn helper() -> std::path::PathBuf {
  env!("CARGO_BIN_EXE_node-auth-token").into()
}

#[cfg(feature = "cli")]
struct Fixture {
  dir: TempDir,
}

#[cfg(feature = "cli")]
impl Fixture {
  fn new() -> Self {
    let dir = tempdir().unwrap();
    for folder in ["project", "home", "config/pnpm"] {
      fs::create_dir_all(dir.path().join(folder)).unwrap();
    }

    fs::write(
      dir.path().join("project/.npmrc"),
      "# project\n@org:registry=https://us-npm.pkg.dev/p/r/\n",
    )
    .unwrap();
    fs::write(
      dir.path().join("home/.npmrc"),
      "# npm credentials stay\n//us-npm.pkg.dev/p/r/:_authToken=old\n",
    )
    .unwrap();

    let bin = dir.path().join("bin");

    fs::create_dir_all(&bin).unwrap();

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;

      fs::write(bin.join("pnpm"), "#!/bin/sh\necho 11.13.0\n").unwrap();
      fs::set_permissions(bin.join("pnpm"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(windows)]
    fs::write(bin.join("pnpm.cmd"), "@echo off\r\necho 11.13.0\r\n").unwrap();

    Self { dir }
  }

  fn command(&self) -> Command {
    self.command_for(env!("CARGO_BIN_EXE_node-auth"))
  }

  fn command_for(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    let mut path = vec![self.dir.path().join("bin")];
    path.extend(std::env::split_paths(
      &std::env::var_os("PATH").unwrap_or_default(),
    ));

    cmd.env("PATH", std::env::join_paths(path).unwrap());

    cmd
      .current_dir(self.dir.path().join("project"))
      .env("HOME", self.dir.path().join("home"))
      .env("USERPROFILE", self.dir.path().join("home"))
      .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
      .env_remove("NODE_AUTH_TOKEN")
      .env_remove("NODE_AUTH_REPO_CONFIG")
      .env_remove("NPM_CONFIG_USERCONFIG")
      .env_remove("pnpm_config_npmrc_auth_file")
      .env_remove("npm_config_userconfig")
      .env_remove("PNPM_CONFIG_USERCONFIG")
      .env_remove("pnpm_config_userconfig")
      .env_remove("PNPM_CONFIG__AUTH")
      .env_remove("pnpm_config__auth");

    #[cfg(windows)]
    cmd.env(
      "PNPM_CONFIG_NPMRC_AUTH_FILE",
      self.dir.path().join("home/.npmrc"),
    );

    cmd
  }
}

#[test]
#[cfg(feature = "cli")]
fn configure_pnpm_writes_only_user_auth_and_is_idempotent() {
  let fixture = Fixture::new();
  let config = fixture.dir.path().join("config/pnpm/auth.ini");

  fs::write(&config, "# keep\n//example.com/:_authToken=other\n").unwrap();

  let output = fixture
    .command()
    .args(["configure", "pnpm"])
    .output()
    .unwrap();
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );

  let content = fs::read_to_string(&config).unwrap();
  assert!(content.starts_with("# keep\n//example.com/:_authToken=other\n"));
  assert!(content.contains("//us-npm.pkg.dev/p/r/:tokenHelper="));

  let path = content
    .lines()
    .find_map(|line| line.strip_prefix("//us-npm.pkg.dev/p/r/:tokenHelper="))
    .unwrap();
  assert!(std::path::Path::new(path).is_absolute());
  assert_eq!(
    fs::canonicalize(path).unwrap(),
    helper().canonicalize().unwrap()
  );
  assert!(!content.contains("old"));

  let modified = fs::metadata(&config).unwrap().modified().unwrap();
  let output = fixture
    .command()
    .args(["configure", "pnpm"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(fs::read_to_string(&config).unwrap(), content);
  assert_eq!(fs::metadata(&config).unwrap().modified().unwrap(), modified);
  assert_eq!(
    fs::read_to_string(fixture.dir.path().join("home/.npmrc")).unwrap(),
    "# npm credentials stay\n//us-npm.pkg.dev/p/r/:_authToken=old\n"
  );
  assert_eq!(
    fs::read_to_string(fixture.dir.path().join("project/.npmrc")).unwrap(),
    "# project\n@org:registry=https://us-npm.pkg.dev/p/r/\n"
  );
}

#[test]
fn discovers_workspace_and_user_registries_from_nested_directory() {
  let f = Fixture::new();
  let project = f.dir.path().join("project");

  fs::write(
    project.join("pnpm-workspace.yaml"),
    "packages:\n  - packages/*\nregistries:\n  '@yaml': https://europe-npm.pkg.dev/p/yaml/\n",
  )
  .unwrap();
  fs::write(
    f.dir.path().join("home/.npmrc"),
    "registry = \"https://asia-npm.pkg.dev/p/user\"\n",
  )
  .unwrap();
  fs::create_dir_all(project.join("packages/app")).unwrap();

  let out = f
    .command()
    .current_dir(project.join("packages/app"))
    .args(["configure", "pnpm"])
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let content = fs::read_to_string(f.dir.path().join("config/pnpm/auth.ini")).unwrap();
  for key in [
    "//us-npm.pkg.dev/p/r/",
    "//europe-npm.pkg.dev/p/yaml/",
    "//asia-npm.pkg.dev/p/user/",
  ] {
    assert!(
      content.contains(&format!("{key}:tokenHelper=")),
      "missing helper for {key} in auth.ini:\n{content}"
    );
  }
}

#[test]
fn preserves_static_credentials_and_reconciles_duplicate_helpers() {
  let f = Fixture::new();
  let config = f.dir.path().join("config/pnpm/auth.ini");
  let before = "# keep\r\n//us-npm.pkg.dev/p/r/:_authToken=old\r\n//us-npm.pkg.dev/p/r:tokenHelper=/old\r\n//us-npm.pkg.dev/p/r/:tokenHelper=/duplicate\r\n//registry.npmjs.org/:_authToken=unrelated\r\n";

  fs::write(&config, before).unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let after = fs::read_to_string(&config).unwrap();
  assert_eq!(after.matches(":tokenHelper=").count(), 1);
  assert!(after.contains("//us-npm.pkg.dev/p/r/:_authToken=old\r\n"));
  assert!(after.contains("//registry.npmjs.org/:_authToken=unrelated\r\n"));
  assert!(after.starts_with("# keep\r\n"));
}

#[test]
fn invalid_configuration_leaves_all_files_unchanged() {
  for source in [
    "registry=http://us-npm.pkg.dev/p/r/\n",
    "registry=https://us-npm.pkg.dev/\n",
    "registry=https://user:secret@us-npm.pkg.dev/p/r/\n",
    "registry=https://registry.npmjs.org/\n",
    "registry=https://us-npm.pkg.dev.evil.test/p/r/\n",
    "registry=${REGISTRY}\n",
    "registry=https://us-npm.pkg.dev/p/r/\n//us-npm.pkg.dev/p/r/child/:_authToken=secret\n",
    "registry=https://us-npm.pkg.dev/p/r/\n//us-npm.pkg.dev/p/r/:@org:_authToken=secret\n",
    "registry=https://us-npm.pkg.dev/p/r/\n//us-npm.pkg.dev/p/r/:tokenHelper=/project-controlled\n",
  ] {
    let f = Fixture::new();

    fs::write(f.dir.path().join("project/.npmrc"), source).unwrap();

    let out = f.command().args(["configure", "pnpm"]).output().unwrap();
    assert!(!out.status.success(), "accepted {source}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("secret"));
    assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
    assert_eq!(
      fs::read_to_string(f.dir.path().join("project/.npmrc")).unwrap(),
      source
    );
  }
}

#[test]
fn refuses_project_auth_destination_and_missing_helper() {
  let f = Fixture::new();
  let out = f
    .command()
    .env("XDG_CONFIG_HOME", f.dir.path().join("project/config"))
    .args(["configure", "pnpm"])
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(!f.dir.path().join("project/config").exists());

  let out = f
    .command()
    .args(["configure", "pnpm", "--token-helper"])
    .arg(f.dir.path().join("missing-helper"))
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
}

#[test]
fn rejects_unsupported_pnpm_and_conflicting_legacy_flags() {
  let f = Fixture::new();

  #[cfg(unix)]
  fs::write(f.dir.path().join("bin/pnpm"), "#!/bin/sh\necho 11.12.0\n").unwrap();

  #[cfg(windows)]
  fs::write(
    f.dir.path().join("bin/pnpm.cmd"),
    "@echo off\r\necho 11.12.0\r\n",
  )
  .unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(!out.status.success());
  assert!(String::from_utf8_lossy(&out.stderr).contains("11.13.0"));

  let out = f
    .command()
    .args(["--local-credential", "configure", "pnpm"])
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
}

#[test]
fn honors_explicit_source_and_pnpm_user_auth_file() {
  let f = Fixture::new();
  let extra = f.dir.path().join("additional.npmrc");
  let user = f.dir.path().join("alternate-user.npmrc");

  fs::write(&extra, "registry=https://europe-npm.pkg.dev/p/extra/\n").unwrap();
  fs::write(&user, "registry=https://asia-npm.pkg.dev/p/alternate/\n").unwrap();

  let out = f
    .command()
    .env("PNPM_CONFIG_NPMRC_AUTH_FILE", &user)
    .args(["configure", "pnpm", "--repo-config"])
    .arg(&extra)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let config = fs::read_to_string(f.dir.path().join("config/pnpm/auth.ini")).unwrap();
  assert!(config.contains("//europe-npm.pkg.dev/p/extra/:tokenHelper="));
  assert!(config.contains("//asia-npm.pkg.dev/p/alternate/:tokenHelper="));
}

#[test]
fn rejects_unrepresentable_helper_path_without_writing() {
  let f = Fixture::new();
  let path = f.dir.path().join(format!(
    "helper with spaces{}",
    std::env::consts::EXE_SUFFIX
  ));

  fs::copy(helper(), &path).unwrap();

  let out = f
    .command()
    .args(["configure", "pnpm", "--token-helper"])
    .arg(path)
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(String::from_utf8_lossy(&out.stderr).contains("path"));
  assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
}

#[test]
#[ignore = "requires installed pnpm >=11.13 and npm; set NODE_AUTH_TEST_PNPM to its executable"]
fn real_package_managers_use_isolated_helper_and_leave_npm_unaffected() {
  let f = Fixture::new();
  let pnpm = std::env::var_os("NODE_AUTH_TEST_PNPM").expect("set NODE_AUTH_TEST_PNPM");

  fs::write(
    f.dir.path().join("config/pnpm/auth.ini"),
    "[unrelated]\nvalue=keep\n",
  )
  .unwrap();
  fs::write(
    f.dir.path().join("project/.npmrc"),
    "@org:registry=https://us-npm.pkg.dev/p/r/ # GAR\n",
  )
  .unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(out.status.success(), "{:?}", out);

  fs::write(
    f.dir.path().join("project/package.json"),
    r#"{"name":"auth-fixture","version":"1.0.0","dependencies":{"@org/does-not-exist":"1.0.0"}}"#,
  )
  .unwrap();

  // An invalid helper token must fail before offline package resolution, despite
  // a valid static token at the same key. No network or Google auth is involved.
  let out = f
    .command_for(&pnpm)
    .env("NODE_AUTH_TOKEN", "invalid token")
    .args(["install", "--offline", "--ignore-scripts"])
    .output()
    .unwrap();
  let diagnostics = format!(
    "{}{}",
    String::from_utf8_lossy(&out.stdout),
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(!out.status.success());
  assert!(
    diagnostics.contains("TOKEN_HELPER_ERROR_STATUS"),
    "{diagnostics}"
  );

  let out = f
    .command_for(&pnpm)
    .env("NODE_AUTH_TOKEN", "fixture-token")
    .args(["install", "--offline", "--ignore-scripts"])
    .output()
    .unwrap();
  let diagnostics = format!(
    "{}{}",
    String::from_utf8_lossy(&out.stdout),
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(!out.status.success()); // No such package in the isolated offline store.
  assert!(diagnostics.contains("NO_OFFLINE_META"), "{diagnostics}");
  assert!(!diagnostics.contains("TOKEN_HELPER"), "{diagnostics}");

  let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
  let out = f
    .command_for(npm)
    .args(["config", "get", "registry"])
    .output()
    .unwrap();
  assert!(out.status.success(), "{:?}", out);
  assert!(
    out.stderr.is_empty(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
}

#[test]
#[ignore = "live GAR access: set NODE_AUTH_TEST_PNPM, NODE_AUTH_GAR_REGISTRY, NODE_AUTH_GAR_PACKAGE and valid ADC"]
fn live_gar_package_metadata_uses_dynamic_authentication() {
  let f = Fixture::new();
  let pnpm = std::env::var_os("NODE_AUTH_TEST_PNPM").expect("set NODE_AUTH_TEST_PNPM");
  let registry = std::env::var("NODE_AUTH_GAR_REGISTRY").expect("set NODE_AUTH_GAR_REGISTRY");
  let package = std::env::var("NODE_AUTH_GAR_PACKAGE").expect("set NODE_AUTH_GAR_PACKAGE");

  fs::write(
    f.dir.path().join("project/.npmrc"),
    format!("registry={registry}\n"),
  )
  .unwrap();
  fs::write(f.dir.path().join("home/.npmrc"), "").unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let mut cmd = f.command_for(pnpm);
  // Authentication may read the developer's real ADC, while all package-manager
  // configuration and output files remain isolated in the fixture.
  for key in ["HOME", "USERPROFILE"] {
    if let Some(value) = std::env::var_os(key) {
      cmd.env(key, value);
    }
  }

  cmd.env("NPM_CONFIG_USERCONFIG", f.dir.path().join("home/.npmrc"));

  let out = cmd
    .args(["view", &package, "version", "--registry", &registry])
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(!out.stdout.is_empty());
}

#[test]
#[cfg(unix)]
fn symlink_cannot_redirect_user_auth_into_project() {
  use std::os::unix::fs::symlink;

  let f = Fixture::new();
  let project_auth = f.dir.path().join("project/secret.ini");

  fs::write(&project_auth, "# untouched\n").unwrap();
  symlink(&project_auth, f.dir.path().join("config/pnpm/auth.ini")).unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(!out.status.success());
  assert_eq!(fs::read_to_string(project_auth).unwrap(), "# untouched\n");
}

#[test]
fn child_registry_definitions_remain_idempotent_with_their_own_helpers() {
  let f = Fixture::new();

  fs::write(f.dir.path().join("project/.npmrc"), "@org:registry=https://us-npm.pkg.dev/p/r/\n@child:registry=https://us-npm.pkg.dev/p/r/child/\n").unwrap();
  fs::write(
    f.dir.path().join("config/pnpm/auth.ini"),
    "//us-npm.pkg.dev/p/r/child:tokenHelper=/old\n",
  )
  .unwrap();

  for _ in 0..2 {
    let out = f.command().args(["configure", "pnpm"]).output().unwrap();
    assert!(
      out.status.success(),
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  let content = fs::read_to_string(f.dir.path().join("config/pnpm/auth.ini")).unwrap();
  assert_eq!(content.matches(":tokenHelper=").count(), 2);
}

#[test]
fn malformed_yaml_and_scoped_structured_auth_do_not_write() {
  for yaml in [
    "registries: [not-a-mapping]\n",
    "registry: [not-a-url]\n",
    "registry: https://us-npm.pkg.dev/p/r/\n_auth:\n  https://us-npm.pkg.dev/p/r/:\n    '@org':\n      authToken: secret\n",
  ] {
    let f = Fixture::new();

    fs::write(f.dir.path().join("config/pnpm/config.yaml"), yaml).unwrap();

    let out = f.command().args(["configure", "pnpm"]).output().unwrap();
    assert!(!out.status.success());
    assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("secret"));
  }
}

#[test]
#[cfg(unix)]
fn new_auth_file_is_private_and_replacements_preserve_permissions() {
  use std::os::unix::fs::PermissionsExt;

  let f = Fixture::new();
  let target = f.dir.path().join("config/pnpm/auth.ini");

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(out.status.success());
  assert_eq!(
    fs::metadata(&target).unwrap().permissions().mode() & 0o777,
    0o600
  );

  fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
  fs::write(&target, "//us-npm.pkg.dev/p/r/:tokenHelper=/old\n").unwrap();

  let out = f.command().args(["configure", "pnpm"]).output().unwrap();
  assert!(out.status.success());
  assert_eq!(
    fs::metadata(target).unwrap().permissions().mode() & 0o777,
    0o640
  );
}

#[test]
#[cfg(unix)]
fn expands_home_in_effective_user_auth_path() {
  let f = Fixture::new();

  fs::write(f.dir.path().join("project/.npmrc"), "# no registry\n").unwrap();
  fs::write(
    f.dir.path().join("home/alternate.npmrc"),
    "registry=https://asia-npm.pkg.dev/p/alternate/\n",
  )
  .unwrap();

  let out = f
    .command()
    .env("PNPM_CONFIG_NPMRC_AUTH_FILE", "~/alternate.npmrc")
    .args(["configure", "pnpm"])
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );

  let content = fs::read_to_string(f.dir.path().join("config/pnpm/auth.ini")).unwrap();
  assert!(content.contains("//asia-npm.pkg.dev/p/alternate/:tokenHelper="));
}

#[test]
fn ini_comments_are_parsed_and_helpers_stay_outside_sections() {
  let f = Fixture::new();

  fs::write(f.dir.path().join("project/.npmrc"), "@org:registry = https://us-npm.pkg.dev/p/r/ # GAR\nregistry=https://asia-npm.pkg.dev/p/r/ ; comment\n[unrelated]\nregistry=https://europe-npm.pkg.dev/p/ignored/\n").unwrap();

  let target = f.dir.path().join("config/pnpm/auth.ini");
  let section = "[unrelated]\nvalue=keep\n//us-npm.pkg.dev/p/r/:tokenHelper=/section-value\n";

  fs::write(&target, format!("# preserve\n{section}")).unwrap();

  for _ in 0..2 {
    let out = f.command().args(["configure", "pnpm"]).output().unwrap();
    assert!(
      out.status.success(),
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  let content = fs::read_to_string(target).unwrap();
  assert!(content.ends_with(section));

  let root = content.split("[unrelated]").next().unwrap();
  assert!(root.contains("//us-npm.pkg.dev/p/r/:tokenHelper="));
  assert!(root.contains("//asia-npm.pkg.dev/p/r/:tokenHelper="));
  assert!(!root.contains("europe-npm.pkg.dev"));
}

#[test]
fn explicit_npmrc_source_is_checked_regardless_of_extension() {
  let f = Fixture::new();
  let source = f.dir.path().join("registries.yaml");

  fs::write(
    &source,
    "registry=https://us-npm.pkg.dev/p/r/\n//us-npm.pkg.dev/p/r/child/:_authToken=secret\n",
  )
  .unwrap();

  let out = f
    .command()
    .args(["configure", "pnpm", "--repo-config"])
    .arg(source)
    .output()
    .unwrap();
  assert!(!out.status.success());
  assert!(!f.dir.path().join("config/pnpm/auth.ini").exists());
}
