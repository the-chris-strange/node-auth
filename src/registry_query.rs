//! Native registry discovery with a short-lived, per-project disk cache.

use crate::AuthError;
use crate::registry::{RegistryPolicy, RegistryUrl};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_AGE: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Manager {
  Npm,
  Pnpm,
  Yarn,
}

impl Manager {
  fn name(self) -> &'static str {
    match self {
      Self::Npm => "npm",
      Self::Pnpm => "pnpm",
      Self::Yarn => "yarn",
    }
  }
}

pub(crate) struct Discovery {
  pub manager: Manager,
  pub modern_yarn: bool,
  pub version: String,
  pub urls: Option<Vec<String>>,
  pub yarn_scopes: BTreeMap<String, String>,
  pub project_root: PathBuf,
  cwd: PathBuf,
  executable: PathBuf,
  cache: Option<PathBuf>,
  created: u64,
}

pub(crate) fn detect(cwd: &Path) -> Result<Manager, AuthError> {
  let home = dirs::home_dir().map(|p| p.canonicalize().unwrap_or(p));
  for directory in cwd.ancestors() {
    if Some(directory) == home.as_deref() {
      break;
    }
    if let Ok(text) = std::fs::read_to_string(directory.join("package.json")) {
      let package: Value = serde_json::from_str(&text)
        .map_err(|_| error("Cannot read packageManager from invalid package.json"))?;
      if let Some(manager) = package.get("packageManager").and_then(Value::as_str) {
        return Ok(match manager.split('@').next() {
          Some("pnpm") => Manager::Pnpm,
          Some("yarn") => Manager::Yarn,
          _ => Manager::Npm,
        });
      }
    }
    if directory.join("pnpm-workspace.yaml").exists() || directory.join("pnpm-lock.yaml").exists() {
      return Ok(Manager::Pnpm);
    }
    if directory.join("yarn.lock").exists()
      || directory.join(".yarnrc.yml").exists()
      || directory.join(".yarnrc").exists()
    {
      return Ok(Manager::Yarn);
    }
    if directory.join("package-lock.json").exists() {
      return Ok(Manager::Npm);
    }
  }
  Ok(Manager::Npm)
}

pub(crate) fn query(manager: Manager) -> Result<Option<Discovery>, AuthError> {
  let cwd = std::env::current_dir()?.canonicalize()?;
  let Some(executable) = executable(manager) else {
    return Ok(None);
  };
  let mut key = DefaultHasher::new();
  cwd.hash(&mut key);
  manager.name().hash(&mut key);
  let cache = std::env::var_os("XDG_CACHE_HOME")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(dirs::cache_dir)
    .map(|p| {
      p.join("node-auth")
        .join(format!("registries-{:016x}.json", key.finish()))
    });
  let mut result = Discovery {
    manager,
    modern_yarn: false,
    version: String::new(),
    urls: None,
    yarn_scopes: BTreeMap::new(),
    project_root: if manager == Manager::Pnpm {
      cwd
        .ancestors()
        .find(|p| p.join("pnpm-workspace.yaml").is_file())
        .or_else(|| cwd.ancestors().find(|p| p.join("package.json").is_file()))
        .unwrap_or(&cwd)
        .to_path_buf()
    } else if manager == Manager::Npm {
      cwd
        .ancestors()
        .find(|p| p.join("package.json").is_file())
        .unwrap_or(&cwd)
        .to_path_buf()
    } else {
      cwd.clone()
    },
    cwd,
    executable,
    cache,
    created: now(),
  };
  let fingerprint = result.fingerprint()?;
  if let Some(cached) = result
    .cache
    .as_ref()
    .and_then(|p| std::fs::read(p).ok())
    .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
    && cached["schema"] == 3
    && cached["fingerprint"].as_u64() == Some(fingerprint)
    && let Some(created) = cached["created"].as_u64()
    && now().checked_sub(created).is_some_and(|age| age < MAX_AGE)
    && let Some(project_root) = cached["projectRoot"].as_str().map(PathBuf::from)
    && project_root.is_absolute()
    && result.cwd.starts_with(&project_root)
    && let Some(version) = cached["version"].as_str()
    && let Some(modern_yarn) = cached["modernYarn"].as_bool()
    && let Ok(yarn_scopes) =
      serde_json::from_value::<BTreeMap<String, String>>(cached["yarnScopes"].clone())
    && (cached["urls"].is_null() || cached["urls"].is_array())
    && let Ok(urls) = if cached["urls"].is_null() {
      Ok(None)
    } else {
      parse_urls(cached["urls"].as_array().unwrap().iter()).map(Some)
    }
  {
    result.project_root = project_root;
    result.version = version.into();
    result.modern_yarn = modern_yarn;
    result.yarn_scopes = yarn_scopes;
    result.urls = urls;
    result.created = created;
    return Ok(Some(result));
  }

  if manager == Manager::Pnpm {
    result.version = result.command(&["--version"])?.trim().to_owned();
    let parts: Vec<_> = result.version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.parse::<u32>().is_err()) {
      return Err(error("Cannot determine stable pnpm version"));
    }
  }
  let args: &[&str] = match manager {
    Manager::Npm => &["config", "list", "--json"],
    Manager::Pnpm => &["config", "get", "registries", "--json"],
    Manager::Yarn => &["config", "--json"],
  };
  let output = match result.command(args) {
    Ok(output) => output,
    // Yarn Classic requires `list`; modern Yarn rejects that positional argument.
    Err(_) if manager == Manager::Yarn => result.command(&["config", "list", "--json"])?,
    Err(error) => return Err(error),
  };
  let (mut urls, modern_yarn) = parse(manager, &output)?;
  if modern_yarn {
    // `yarn config --json` serializes Map settings as {}, so query scopes explicitly.
    let scopes: Value =
      serde_json::from_str(&result.command(&["config", "get", "npmScopes", "--json"])?)
        .map_err(|_| error("Yarn returned invalid npmScopes JSON"))?;
    let scopes = scopes
      .as_object()
      .ok_or_else(|| error("Yarn npmScopes must be an object"))?;
    let default = urls
      .as_ref()
      .unwrap()
      .first()
      .ok_or_else(|| error("Yarn did not report its default registry"))?
      .clone();
    for (name, scope) in scopes {
      if !scope.is_object() {
        return Err(error("Yarn scope configuration must be an object"));
      }
      let registry = scope
        .get("npmRegistryServer")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| Value::String(default.clone()));
      let registry = parse_urls(std::iter::once(&registry))?.pop().unwrap();
      result.yarn_scopes.insert(name.clone(), registry.clone());
      urls.as_mut().unwrap().push(registry);
    }
  }
  result.urls = urls;
  result.modern_yarn = modern_yarn;
  if result.fingerprint()? != fingerprint {
    return Err(error(
      "Package-manager configuration changed during registry discovery; run again",
    ));
  }
  Ok(Some(result))
}

impl Discovery {
  pub fn registries(&self, allow_all_domains: bool) -> Result<Option<Vec<RegistryUrl>>, AuthError> {
    self
      .urls
      .as_ref()
      .map(|urls| {
        let policy = RegistryPolicy { allow_all_domains };
        urls
          .iter()
          .filter_map(|url| policy.parse(url).transpose())
          .collect()
      })
      .transpose()
  }

  // Save after credential writes, which otherwise invalidate our own cache.
  pub fn save(&self) {
    let Some(path) = &self.cache else { return };
    let save = || -> Result<(), Box<dyn std::error::Error>> {
      std::fs::create_dir_all(path.parent().unwrap())?;
      let data = json!({"schema": 3, "fingerprint": self.fingerprint()?, "created": self.created,
        "version": self.version, "modernYarn": self.modern_yarn, "urls": self.urls, "yarnScopes": self.yarn_scopes, "projectRoot": self.project_root});
      let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
      temp.write_all(data.to_string().as_bytes())?;
      temp.persist(path)?;
      Ok(())
    };
    if save().is_err() {
      log::debug!("Could not save registry discovery cache");
    }
  }

  fn fingerprint(&self) -> Result<u64, AuthError> {
    let mut hash = DefaultHasher::new();
    self.executable.hash(&mut hash);
    let metadata = std::fs::metadata(&self.executable)?;
    metadata.len().hash(&mut hash);
    metadata.modified()?.hash(&mut hash);
    let mut files = BTreeSet::new();
    for directory in self
      .cwd
      .ancestors()
      .map(Path::to_path_buf)
      .chain(dirs::home_dir())
    {
      for name in [
        ".npmrc",
        ".yarnrc",
        ".yarnrc.yml",
        "pnpm-workspace.yaml",
        "package.json",
      ] {
        files.insert(directory.join(name));
      }
    }
    if let Some(home) = dirs::home_dir() {
      files.insert(home.join(".config/yarn/config"));
    }
    if let Some(home) = dirs::home_dir() {
      let config = crate::pnpm::config_dir(&home)?;
      for name in ["config.yaml", "rc", "auth.ini"] {
        files.insert(config.join(name));
      }
    }
    let prefix = self.executable.parent().and_then(|directory| {
      if cfg!(windows) {
        Some(directory)
      } else {
        directory.parent()
      }
    });
    if let Some(prefix) = prefix {
      files.insert(prefix.join("etc/npmrc"));
    }
    let mut environment: Vec<_> = std::env::vars_os().collect();
    environment.sort();
    for (key, value) in &environment {
      let key = key.to_string_lossy().to_ascii_lowercase();
      if [
        "npm_config_userconfig",
        "npm_config_globalconfig",
        "pnpm_config_npmrc_auth_file",
      ]
      .contains(&key.as_str())
      {
        files.insert(PathBuf::from(value));
      }
    }
    environment.hash(&mut hash);
    for path in files {
      path.hash(&mut hash);
      match std::fs::read(&path) {
        Ok(contents) => contents.hash(&mut hash),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0u8.hash(&mut hash),
        Err(e) => return Err(e.into()),
      }
    }
    Ok(hash.finish())
  }

  fn command(&mut self, args: &[&str]) -> Result<String, AuthError> {
    let directory = self.cwd.clone();
    self.command_in(args, &directory)
  }

  fn command_in(&mut self, args: &[&str], directory: &Path) -> Result<String, AuthError> {
    let mut child = Command::new(&self.executable)
      .args(args)
      .current_dir(directory)
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
      let mut bytes = Vec::new();
      stdout
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
      if let Some(status) = child.try_wait()? {
        break Some(status);
      }
      if start.elapsed() > Duration::from_secs(15) {
        let _ = child.kill();
        let _ = child.wait();
        break None;
      }
      std::thread::sleep(Duration::from_millis(20));
    };
    // Do not expose config output or stderr: both may contain credentials.
    let failed = || {
      error(&format!(
        "{} registry query failed or timed out; check its configuration or use an explicit --repo-config source",
        self.manager.name()
      ))
    };
    let status = status.ok_or_else(failed)?;
    let bytes = reader
      .join()
      .map_err(|_| error("Registry query output could not be read"))??;
    if bytes.len() > 4 * 1024 * 1024 {
      return Err(error("Registry query output is too large"));
    }
    if !status.success() {
      if self.manager == Manager::Npm
        && serde_json::from_slice::<Value>(&bytes)
          .ok()
          .is_some_and(|value| value["error"]["code"] == "ENOWORKSPACES")
        && let Some(root) = directory.ancestors().skip(1).find(|root| {
          std::fs::read(root.join("package.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|package| package.get("workspaces").is_some())
        })
      {
        return self.command_in(args, root);
      }
      return Err(failed());
    }
    if self.manager == Manager::Npm && directory != self.cwd {
      self.project_root = directory.to_path_buf();
    }
    String::from_utf8(bytes).map_err(|_| error("Registry query output must be UTF-8"))
  }
}

fn executable(manager: Manager) -> Option<PathBuf> {
  let names = if cfg!(windows) {
    vec![
      format!("{}.cmd", manager.name()),
      format!("{}.exe", manager.name()),
      format!("{}.bat", manager.name()),
    ]
  } else {
    vec![manager.name().to_owned()]
  };
  std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
    .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
    .find(|p| p.is_file())
    .and_then(|p| std::path::absolute(p).ok())
}

fn parse(manager: Manager, output: &str) -> Result<(Option<Vec<String>>, bool), AuthError> {
  let invalid = || {
    error(
      "Package manager returned an unsupported registry configuration; use an explicit --repo-config source to read files instead",
    )
  };
  if manager == Manager::Pnpm && output.trim() == "undefined" {
    return Ok((None, false));
  }
  let mut urls = Vec::new();
  match manager {
    Manager::Npm | Manager::Pnpm => {
      let value: Value = serde_json::from_str(output).map_err(|_| invalid())?;
      let entries = value.as_object().ok_or_else(invalid)?;
      for (key, value) in entries {
        if manager == Manager::Pnpm {
          if key.starts_with("https://") || key.starts_with("http://") {
            if !value.is_object() {
              return Err(invalid());
            }
            urls.push(Value::String(key.clone()));
          } else {
            urls.push(value.clone());
          }
        } else if key == "registry" || (key.starts_with('@') && key.ends_with(":registry")) {
          urls.push(value.clone());
        }
      }
    }
    Manager::Yarn => {
      let mut modern = false;
      let mut classic = false;
      let mut configs = BTreeMap::new();
      for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let entry: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        if let Some(key) = entry["key"].as_str() {
          modern = true;
          if key == "npmRegistryServer" {
            urls.push(entry["effective"].clone());
          }
        } else if entry["type"] == "inspect" {
          classic = true;
          for (key, value) in entry["data"].as_object().ok_or_else(invalid)? {
            if key == "registry" || (key.starts_with('@') && key.ends_with(":registry")) {
              configs.insert(key.clone(), value.clone());
            }
          }
        } else if entry["type"] != "info"
          && entry["type"] != "success"
          && entry["type"] != "warning"
        {
          return Err(invalid());
        }
      }
      if !modern && !classic {
        return Err(invalid());
      }
      urls.extend(configs.into_values());
      return Ok((Some(parse_urls(urls.iter())?), modern));
    }
  }
  Ok((Some(parse_urls(urls.iter())?), false))
}

fn parse_urls<'a>(values: impl Iterator<Item = &'a Value>) -> Result<Vec<String>, AuthError> {
  let policy = RegistryPolicy {
    allow_all_domains: true,
  };
  let urls: Result<BTreeSet<_>, AuthError> = values
    .map(|value| {
      let url = value
        .as_str()
        .ok_or_else(|| error("Registry query URLs must be strings"))?;
      Ok(policy.parse(url)?.unwrap().as_url().to_owned())
    })
    .collect();
  Ok(urls?.into_iter().collect())
}

fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn error(message: &str) -> AuthError {
  AuthError::Config(message.into())
}
