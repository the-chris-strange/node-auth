//! Package-manager fixtures keep discovery and persistent-cache tests offline.
#![cfg(all(feature = "cli", unix))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::{TempDir, tempdir};

struct Fixture {
  dir: TempDir,
}

impl Fixture {
  fn new(manager: &str, response: &str) -> Self {
    let dir = tempdir().unwrap();
    for folder in ["project", "home", "config", "cache", "bin"] {
      fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    fs::write(
      dir.path().join("project/package.json"),
      format!(r#"{{"packageManager":"{manager}@11.23.0"}}"#),
    )
    .unwrap();
    fs::write(dir.path().join("response"), response).unwrap();
    let executable = dir.path().join("bin").join(manager);
    fs::write(&executable, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 11.23.0; exit; fi\necho query >> \"$FIXTURE_ROOT/queries\"\nif [ \"$3\" = npmScopes ]; then cat \"$FIXTURE_ROOT/scopes\"; else cat \"$FIXTURE_ROOT/response\"; fi\n").unwrap();
    fs::set_permissions(executable, fs::Permissions::from_mode(0o755)).unwrap();
    Self { dir }
  }

  fn command(&self) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_node-auth"));
    for (key, _) in std::env::vars_os() {
      if ["npm_config_", "pnpm_config_", "yarn_", "node_auth_"]
        .iter()
        .any(|p| key.to_string_lossy().to_ascii_lowercase().starts_with(p))
      {
        cmd.env_remove(key);
      }
    }
    cmd
      .current_dir(self.dir.path().join("project"))
      .env("HOME", self.dir.path().join("home"))
      .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
      .env("XDG_CACHE_HOME", self.dir.path().join("cache"))
      .env("FIXTURE_ROOT", self.dir.path())
      .env(
        "PATH",
        format!("{}:/usr/bin:/bin", self.dir.path().join("bin").display()),
      );
    cmd
  }

  fn run(&self) {
    let output = self
      .command()
      .args(["--token", "fresh-token"])
      .output()
      .unwrap();
    assert!(
      output.status.success(),
      "{}",
      String::from_utf8_lossy(&output.stderr)
    );
  }

  fn queries(&self) -> usize {
    fs::read_to_string(self.dir.path().join("queries"))
      .unwrap()
      .lines()
      .count()
  }

  fn credentials(&self) -> String {
    fs::read_to_string(self.dir.path().join("home/.npmrc")).unwrap()
  }
}

#[test]
fn npm_queries_effective_routes_and_reuses_cache_after_credential_writes() {
  let f = Fixture::new(
    "npm",
    r#"{"registry":"https://registry.npmjs.org/","@org:registry":"https://us-npm.pkg.dev/p/effective/","//us-npm.pkg.dev/p/effective/:_authToken":"secret-from-query"}"#,
  );
  fs::write(
    f.dir.path().join("project/.npmrc"),
    "@org:registry=https://us-npm.pkg.dev/p/overridden/\n",
  )
  .unwrap();
  f.run();
  f.run();
  assert_eq!(f.queries(), 1);
  assert!(
    f.credentials()
      .contains("//us-npm.pkg.dev/p/effective/:_authToken=\"fresh-token\"")
  );
  assert!(!f.credentials().contains("overridden"));
  for entry in fs::read_dir(f.dir.path().join("cache/node-auth")).unwrap() {
    let text = fs::read_to_string(entry.unwrap().path()).unwrap();
    assert!(!text.contains("secret-from-query"));
    assert!(!text.contains("fresh-token"));
  }
  fs::write(f.dir.path().join("project/.npmrc"), "save-exact=true\n").unwrap();
  f.run();
  assert_eq!(f.queries(), 2);
  let output = f
    .command()
    .env("npm_config_registry", "https://another.example/")
    .args(["--token", "fresh-token"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(f.queries(), 3);
}

#[test]
fn pnpm_queries_workspace_registry_for_static_authentication() {
  for response in [
    r#"{"default":"https://registry.npmjs.org/","@org":"https://us-npm.pkg.dev/p/workspace/"}"#,
    r#"{"https://registry.npmjs.org/":{"scopes":["@"]},"https://us-npm.pkg.dev/p/workspace/":{"scopes":["@org"]}}"#,
  ] {
    let f = Fixture::new("pnpm", response);
    fs::write(
      f.dir.path().join("project/pnpm-workspace.yaml"),
      "packages: []\n",
    )
    .unwrap();
    f.run();
    f.run();
    assert_eq!(f.queries(), 1);
    assert!(
      f.credentials()
        .contains("//us-npm.pkg.dev/p/workspace/:_authToken=")
    );
  }
}

#[test]
fn modern_yarn_queries_default_and_scoped_registries_without_changing_routing() {
  let f = Fixture::new(
    "yarn",
    "{\"key\":\"npmRegistryServer\",\"effective\":\"https://us-npm.pkg.dev/p/default/\"}\n{\"key\":\"npmScopes\",\"effective\":{}}\n",
  );
  fs::write(f.dir.path().join("scopes"), r#"{"org":{"npmRegistryServer":"https://europe-npm.pkg.dev/p/scoped/","npmAuthToken":"secret-from-query"}}"#).unwrap();
  fs::write(
    f.dir.path().join("home/.yarnrc.yml"),
    "npmScopes:\n  org:\n    npmAuthToken: expired-scope-token\n",
  )
  .unwrap();
  f.run();
  f.run();
  assert_eq!(f.queries(), 2);
  let credentials = fs::read_to_string(f.dir.path().join("home/.yarnrc.yml")).unwrap();
  assert!(credentials.contains("npmRegistries:"));
  assert!(credentials.contains("https://us-npm.pkg.dev/p/default/"));
  assert!(credentials.contains("https://europe-npm.pkg.dev/p/scoped/"));
  assert!(credentials.contains("npmAuthToken: fresh-token"));
  assert!(!credentials.contains("expired-scope-token"));
  assert!(!credentials.contains("secret-from-query"));
}

#[test]
fn classic_yarn_discovers_scoped_npm_configuration() {
  let f = Fixture::new(
    "yarn",
    "{\"type\":\"info\",\"data\":\"yarn config\"}\n{\"type\":\"inspect\",\"data\":{\"registry\":\"https://registry.yarnpkg.com\"}}\n{\"type\":\"info\",\"data\":\"npm config\"}\n{\"type\":\"inspect\",\"data\":{\"@org:registry\":\"https://us-npm.pkg.dev/p/classic/\"}}\n",
  );
  fs::write(f.dir.path().join("bin/yarn"), "#!/bin/sh\necho query >> \"$FIXTURE_ROOT/queries\"\nif [ \"$2\" != list ]; then exit 1; fi\ncat \"$FIXTURE_ROOT/response\"\n").unwrap();
  f.run();
  f.run();
  assert_eq!(f.queries(), 2);
  assert!(
    f.credentials()
      .contains("//us-npm.pkg.dev/p/classic/:_authToken=")
  );
  assert!(!f.dir.path().join("home/.yarnrc.yml").exists());
}

#[test]
fn malformed_query_output_does_not_fall_back_or_write_credentials() {
  let f = Fixture::new("npm", "not JSON: secret-from-query");
  fs::write(
    f.dir.path().join("project/.npmrc"),
    "registry=https://us-npm.pkg.dev/p/file/\n",
  )
  .unwrap();
  let output = f
    .command()
    .args(["--token", "fresh-token"])
    .output()
    .unwrap();
  assert!(!output.status.success());
  assert!(!f.dir.path().join("home/.npmrc").exists());
  assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-from-query"));
}

#[test]
fn pnpm_helper_setup_uses_and_caches_resolved_registries() {
  let f = Fixture::new("pnpm", r#"{"@org":"https://us-npm.pkg.dev/p/helper/"}"#);
  for _ in 0..2 {
    let output = f.command().args(["configure", "pnpm"]).output().unwrap();
    assert!(
      output.status.success(),
      "{}",
      String::from_utf8_lossy(&output.stderr)
    );
  }
  assert_eq!(f.queries(), 1);
  let auth = fs::read_to_string(f.dir.path().join("config/pnpm/auth.ini")).unwrap();
  assert!(auth.contains("//us-npm.pkg.dev/p/helper/:tokenHelper="));
}

#[test]
fn expired_and_corrupt_caches_are_requeried() {
  let f = Fixture::new("npm", r#"{"registry":"https://us-npm.pkg.dev/p/cache/"}"#);
  f.run();
  let cache = fs::read_dir(f.dir.path().join("cache/node-auth"))
    .unwrap()
    .next()
    .unwrap()
    .unwrap()
    .path();
  let mut entry: serde_json::Value =
    serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
  entry["created"] = 0.into();
  fs::write(&cache, entry.to_string()).unwrap();
  f.run();
  assert_eq!(f.queries(), 2);
  fs::write(&cache, "broken JSON").unwrap();
  f.run();
  assert_eq!(f.queries(), 3);
  let mut entry: serde_json::Value =
    serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
  entry["urls"] = serde_json::json!([42]);
  fs::write(&cache, entry.to_string()).unwrap();
  f.run();
  assert_eq!(f.queries(), 4);
}

#[test]
fn executable_changes_and_failed_queries_do_not_reuse_stale_results() {
  let f = Fixture::new("npm", r#"{"registry":"https://us-npm.pkg.dev/p/cache/"}"#);
  f.run();
  let executable = f.dir.path().join("bin/npm");
  fs::write(
    &executable,
    "#!/bin/sh\necho query >> \"$FIXTURE_ROOT/queries\"\necho secret-from-query >&2\nexit 1\n",
  )
  .unwrap();
  let output = f.command().args(["--token", "new-token"]).output().unwrap();
  assert!(!output.status.success());
  assert_eq!(f.queries(), 2);
  assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-from-query"));
  assert!(!f.credentials().contains("new-token"));
}

#[test]
fn global_yarn_configuration_does_not_select_yarn_for_an_npm_project() {
  let f = Fixture::new("npm", r#"{"registry":"https://us-npm.pkg.dev/p/npm/"}"#);
  fs::write(
    f.dir.path().join("project/package.json"),
    "{\"private\":true}",
  )
  .unwrap();
  fs::write(f.dir.path().join(".yarnrc.yml"), "nodeLinker: pnp\n").unwrap();
  let result = f
    .command()
    .env("HOME", f.dir.path())
    .args(["--token", "fresh-token"])
    .output()
    .unwrap();
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  assert_eq!(f.queries(), 1);
}

#[test]
fn explicit_sources_and_missing_managers_keep_file_discovery() {
  let f = Fixture::new("npm", "not JSON");
  let source = f.dir.path().join("project/.npmrc");
  fs::write(&source, "registry=https://us-npm.pkg.dev/p/files/\n").unwrap();
  let result = f
    .command()
    .arg("--repo-config")
    .arg(&source)
    .args(["--token", "fresh-token"])
    .output()
    .unwrap();
  assert!(result.status.success());
  assert!(!f.dir.path().join("queries").exists());
  fs::remove_file(f.dir.path().join("bin/npm")).unwrap();
  let result = f
    .command()
    .env("PATH", f.dir.path().join("bin"))
    .args(["--token", "fresh-token"])
    .output()
    .unwrap();
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  assert!(
    f.credentials()
      .contains("//us-npm.pkg.dev/p/files/:_authToken=")
  );
}

#[test]
fn cache_write_failure_does_not_prevent_authentication() {
  let f = Fixture::new("npm", r#"{"registry":"https://us-npm.pkg.dev/p/cache/"}"#);
  fs::write(f.dir.path().join("cache/node-auth"), "not a directory").unwrap();
  f.run();
  f.run();
  assert_eq!(f.queries(), 2);
}

#[test]
fn query_results_keep_registry_validation_and_domain_policy() {
  for url in [
    "http://us-npm.pkg.dev/p/r/",
    "https://user:password@us-npm.pkg.dev/p/r/",
    "https://us-npm.pkg.dev/missing-repository",
    "https://us-npm.pkg.dev.evil.test/p/r/",
  ] {
    let f = Fixture::new("npm", &serde_json::json!({"registry": url}).to_string());
    let result = f
      .command()
      .args(["--token", "fresh-token"])
      .output()
      .unwrap();
    assert!(!result.status.success());
    assert!(!f.dir.path().join("home/.npmrc").exists());
  }
  let f = Fixture::new("npm", r#"{"registry":"https://private.example/npm/"}"#);
  let result = f
    .command()
    .args(["--token", "fresh-token", "--allow-all-domains"])
    .output()
    .unwrap();
  assert!(result.status.success());
  assert!(
    f.credentials()
      .contains("//private.example/npm/:_authToken=")
  );
}

#[test]
fn arbitrary_registry_environment_changes_invalidate_cache() {
  let f = Fixture::new("npm", "{}");
  fs::write(f.dir.path().join("bin/npm"), "#!/bin/sh\necho query >> \"$FIXTURE_ROOT/queries\"\necho \"{\\\"registry\\\":\\\"https://${REGION}-npm.pkg.dev/p/env/\\\"}\"\n").unwrap();
  fs::write(
    f.dir.path().join("project/.npmrc"),
    "registry=https://${REGION}-npm.pkg.dev/p/env/\n",
  )
  .unwrap();
  for region in ["us", "europe"] {
    let result = f
      .command()
      .env("REGION", region)
      .args(["--token", "fresh-token"])
      .output()
      .unwrap();
    assert!(
      result.status.success(),
      "{}",
      String::from_utf8_lossy(&result.stderr)
    );
  }
  assert_eq!(f.queries(), 2);
  assert!(
    f.credentials()
      .contains("//europe-npm.pkg.dev/p/env/:_authToken=")
  );
}

#[test]
fn nested_npm_workspace_queries_root_and_moves_expired_project_token() {
  let f = Fixture::new("npm", "{}");
  let project = f.dir.path().join("project");
  fs::write(
    project.join("package.json"),
    r#"{"private":true,"workspaces":["packages/*"],"packageManager":"npm@11.19.0"}"#,
  )
  .unwrap();
  let nested = project.join("packages/app");
  fs::create_dir_all(&nested).unwrap();
  fs::write(nested.join("package.json"), "{\"name\":\"app\"}").unwrap();
  fs::write(project.join(".npmrc"), "registry=https://us-npm.pkg.dev/p/workspace/\n//us-npm.pkg.dev/p/workspace/:_authToken=expired\n").unwrap();
  fs::write(
    f.dir.path().join("response"),
    serde_json::json!({"registry":"https://us-npm.pkg.dev/p/workspace/"}).to_string(),
  )
  .unwrap();
  fs::write(f.dir.path().join("bin/npm"), "#!/bin/sh\necho query >> \"$FIXTURE_ROOT/queries\"\nif [ ! -d packages/app ]; then echo '{\"error\":{\"code\":\"ENOWORKSPACES\"}}'; exit 1; fi\ncat \"$FIXTURE_ROOT/response\"\n").unwrap();
  for _ in 0..2 {
    let result = f
      .command()
      .current_dir(&nested)
      .args(["--token", "fresh-token"])
      .output()
      .unwrap();
    assert!(
      result.status.success(),
      "{}",
      String::from_utf8_lossy(&result.stderr)
    );
  }
  assert_eq!(f.queries(), 2);
  assert!(
    !fs::read_to_string(project.join(".npmrc"))
      .unwrap()
      .contains("expired")
  );
  assert!(
    f.credentials()
      .contains("//us-npm.pkg.dev/p/workspace/:_authToken=\"fresh-token\"")
  );
}

#[test]
fn nested_pnpm_workspace_moves_expired_root_token() {
  let f = Fixture::new("pnpm", r#"{"@org":"https://us-npm.pkg.dev/p/workspace/"}"#);
  let project = f.dir.path().join("project");
  fs::write(
    project.join("pnpm-workspace.yaml"),
    "packages: ['packages/*']\n",
  )
  .unwrap();
  fs::write(project.join(".npmrc"), "@org:registry=https://us-npm.pkg.dev/p/workspace/\n//us-npm.pkg.dev/p/workspace/:_authToken=expired\n").unwrap();
  let nested = project.join("packages/app");
  fs::create_dir_all(&nested).unwrap();
  fs::write(nested.join("package.json"), "{}").unwrap();
  for _ in 0..2 {
    let result = f
      .command()
      .current_dir(&nested)
      .args(["--token", "fresh-token"])
      .output()
      .unwrap();
    assert!(
      result.status.success(),
      "{}",
      String::from_utf8_lossy(&result.stderr)
    );
  }
  assert_eq!(f.queries(), 1);
  assert!(
    !fs::read_to_string(project.join(".npmrc"))
      .unwrap()
      .contains("expired")
  );
  assert!(
    f.credentials()
      .contains("//us-npm.pkg.dev/p/workspace/:_authToken=\"fresh-token\"")
  );
}
