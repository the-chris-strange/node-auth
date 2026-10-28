//! Dynamic pnpm authentication in the trusted, pnpm-specific user auth file.

use crate::AuthError;
use crate::fs::{FileTransaction, resolve};
use crate::registry::RegistryPolicy;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use yaml_edit::YamlFile;

/// Configure pnpm without retrieving credentials or editing shared npm files.
///
/// Requires pnpm >=11.13.0, whose `auth.ini` supports trusted token helpers.
/// `source` optionally selects an additional `.npmrc` registry source.
/// `helper` optionally selects an installed executable instead of sibling/PATH discovery.
/// Returns the auth file and whether it changed. Project configuration is read only.
///
/// # Errors
/// Returns an error for unsupported pnpm versions, invalid registries, unsafe helper
/// paths, project-local destinations, or conflicting more-specific credentials.
pub fn configure(
  source: Option<&Path>,
  helper: Option<&Path>,
) -> Result<(PathBuf, bool), AuthError> {
  let discovery =
    crate::registry_query::query(crate::registry_query::Manager::Pnpm)?.ok_or_else(|| {
      config_error("pnpm is missing; install pnpm >=11.13.0 and ensure it is on PATH")
    })?;
  check_version(&discovery.version)?;

  let home = dirs::home_dir().ok_or_else(|| config_error("Cannot determine home directory"))?;
  let cwd = std::env::current_dir()?.canonicalize()?;
  let root = cwd
    .ancestors()
    .find(|p| p.join("pnpm-workspace.yaml").is_file())
    .or_else(|| cwd.ancestors().find(|p| p.join("package.json").is_file()))
    .unwrap_or(&cwd);
  let config_dir = config_dir(&home)?;
  let target = resolve(&config_dir.join("auth.ini"))?;

  if target.starts_with(root) {
    return Err(config_error(
      "pnpm auth.ini resolves inside the project; choose a user-level XDG_CONFIG_HOME",
    ));
  }

  let helper = resolve_helper(helper)?;
  let global_yaml = config_dir.join("config.yaml");
  let global_content = read_optional(&global_yaml)?;
  let user = user_npmrc(&home, &global_content)?;
  let user = if let Some(raw) = user
    .to_str()
    .and_then(|p| p.strip_prefix("~/").or_else(|| p.strip_prefix(r"~\")))
  {
    home.join(raw)
  } else {
    user
  };

  let mut sources = vec![resolve(&user)?, resolve(&root.join(".npmrc"))?];

  if let Some(source) = source {
    if !source.is_file() {
      return Err(config_error(
        "Registry source does not exist; check --repo-config",
      ));
    }

    sources.push(resolve(source)?);
  }

  if sources.contains(&target) {
    return Err(config_error("Registry source must not be pnpm auth.ini"));
  }

  sources.sort();
  sources.dedup();

  let mut paths = sources.clone();
  paths.push(target.clone());
  paths.push(resolve(&global_yaml)?);

  let workspace_yaml = resolve(&root.join("pnpm-workspace.yaml"))?;
  paths.push(workspace_yaml.clone());

  let mut transaction = FileTransaction::new(&paths)?;
  let mut registries = BTreeSet::new();

  let global = transaction.contents(&resolve(&global_yaml)?)?;

  if global != global_content {
    return Err(config_error(
      "Global pnpm configuration changed during discovery; run configure again",
    ));
  }

  let project_npmrc = resolve(&root.join(".npmrc"))?;
  if project_npmrc != resolve(&user)?
    && root_ini_lines(transaction.contents(&project_npmrc)?).any(|line| {
      ini_entry(line).is_some_and(|(key, _)| key == "tokenHelper" || key.ends_with(":tokenHelper"))
    })
  {
    return Err(config_error(
      "Project .npmrc contains tokenHelper, which pnpm rejects; remove it from the project before configuring the trusted user auth.ini",
    ));
  }

  if let Some(resolved) = discovery.registries(false)? {
    registries.extend(resolved.iter().map(|r| r.npm_key().to_owned()));
    if let Some(source) = source {
      discover_npmrc(transaction.contents(&resolve(source)?)?, &mut registries)?;
    }
  } else {
    for path in &sources {
      discover_npmrc(transaction.contents(path)?, &mut registries)?;
    }
    discover_yaml(global, &mut registries)?;
    discover_yaml(transaction.contents(&workspace_yaml)?, &mut registries)?;
    discover_npmrc(transaction.contents(&target)?, &mut registries)?;
  }

  if registries.is_empty() {
    return Err(config_error(
      "No GAR registry definitions found; set registry or @scope:registry in .npmrc, or registry/registries in pnpm-workspace.yaml",
    ));
  }

  for path in sources.iter().chain(std::iter::once(&target)) {
    check_conflicts(transaction.contents(path)?, &registries)?;
  }

  // Scope-specific JSON/YAML credentials can override a registry-wide helper.
  check_yaml_auth(global, &registries)?;

  for (key, _) in std::env::vars_os() {
    let key = key.to_string_lossy();
    if let Some((prefix, scoped)) = key.split_once("//")
      && ["npm_config_", "pnpm_config_"]
        .iter()
        .any(|p| prefix.eq_ignore_ascii_case(p))
    {
      check_conflicts(&format!("//{scoped}=environment"), &registries)?;
    }

    if key.eq_ignore_ascii_case("pnpm_config__auth") {
      return Err(config_error(
        "pnpm_config__auth is set; remove the structured auth override before configuring dynamic GAR authentication",
      ));
    }
  }
  let original = transaction.contents(&target)?;
  let updated = transform(original, &registries, &helper)?;

  if updated == original {
    discovery.save();
    return Ok((target, false));
  }

  transaction.stage(&target, &updated)?;
  transaction.commit()?;
  discovery.save();

  Ok((target, true))
}

fn config_error(message: &str) -> AuthError {
  AuthError::Config(message.into())
}

fn check_version(version: &str) -> Result<(), AuthError> {
  let version = version.trim();
  let parts: Vec<_> = version.split('.').map(str::parse::<u32>).collect();

  if parts.len() != 3 || parts.iter().any(Result::is_err) {
    return Err(config_error(
      "Cannot determine stable pnpm version; install pnpm >=11.13.0",
    ));
  }

  let values: Vec<_> = parts.into_iter().map(Result::unwrap).collect();

  if values.as_slice() < [11, 13, 0].as_slice() {
    return Err(config_error(
      "Dynamic configuration requires pnpm >=11.13.0 (trusted auth.ini tokenHelper fix); upgrade pnpm or use node-auth for static credentials",
    ));
  }

  Ok(())
}

pub(crate) fn config_dir(home: &Path) -> Result<PathBuf, AuthError> {
  let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
    let path = PathBuf::from(xdg);
    if !path.is_absolute() {
      return Err(config_error("XDG_CONFIG_HOME must be absolute"));
    }

    path.join("pnpm")
  } else if cfg!(target_os = "macos") {
    home.join("Library/Preferences/pnpm")
  } else if cfg!(windows) {
    std::env::var_os("LOCALAPPDATA")
      .filter(|v| !v.is_empty())
      .map(|p| PathBuf::from(p).join("pnpm/config"))
      .unwrap_or_else(|| home.join(".config/pnpm"))
  } else {
    home.join(".config/pnpm")
  };

  resolve(&base)
}

fn read_optional(path: &Path) -> Result<String, AuthError> {
  match std::fs::read_to_string(path) {
    Ok(s) => Ok(s),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
    Err(e) => Err(e.into()),
  }
}

fn yaml_root(content: &str) -> Result<Option<yaml_edit::Mapping>, AuthError> {
  if content.trim().is_empty() {
    return Ok(None);
  }

  let file = YamlFile::from_str(content).map_err(|source| AuthError::YamlInput {
    input: "pnpm configuration",
    source,
  })?;

  let doc = file
    .document()
    .ok_or_else(|| config_error("pnpm YAML must contain a mapping"))?;

  Ok(Some(doc.as_mapping().ok_or_else(|| {
    config_error("pnpm YAML must contain a mapping")
  })?))
}

fn user_npmrc(home: &Path, global: &str) -> Result<PathBuf, AuthError> {
  for key in [
    "pnpm_config_npmrc_auth_file",
    "PNPM_CONFIG_NPMRC_AUTH_FILE",
    "pnpm_config_userconfig",
    "PNPM_CONFIG_USERCONFIG",
  ] {
    if let Some(value) = std::env::var_os(key).filter(|v| !v.is_empty()) {
      return Ok(PathBuf::from(value));
    }
  }

  if let Some(root) = yaml_root(global)?
    && let Some(value) = root.get("npmrcAuthFile")
  {
    let value = value
      .as_scalar()
      .ok_or_else(|| config_error("npmrcAuthFile must be a path string"))?
      .as_string();
    if value.contains("${") {
      return Err(config_error(
        "Expand npmrcAuthFile to an absolute path before configuring pnpm",
      ));
    }

    return Ok(if let Some(rest) = value.strip_prefix("~/") {
      home.join(rest)
    } else {
      PathBuf::from(value)
    });
  }

  Ok(
    std::env::var_os("npm_config_userconfig")
      .or_else(|| std::env::var_os("NPM_CONFIG_USERCONFIG"))
      .map(PathBuf::from)
      .unwrap_or_else(|| home.join(".npmrc")),
  )
}

fn add_registry(value: &str, registries: &mut BTreeSet<String>) -> Result<(), AuthError> {
  if value.contains("${") {
    return Err(config_error(
      "Registry URL contains an unresolved environment variable; use a literal HTTPS GAR URL",
    ));
  }

  if let Some(registry) = (RegistryPolicy {
    allow_all_domains: false,
  })
  .parse(value)?
  {
    registries.insert(registry.npm_key().to_owned());
  }

  Ok(())
}

fn ini_entry(line: &str) -> Option<(&str, &str)> {
  let line = line.trim();

  if line.starts_with(['#', ';']) {
    return None;
  }

  line.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
}

// npm INI sections form nested objects; registry/auth settings must be root keys.
fn root_ini_lines(content: &str) -> impl Iterator<Item = &str> {
  content
    .lines()
    .take_while(|line| !line.trim_start().starts_with('['))
}

fn ini_value(value: &str) -> Result<String, AuthError> {
  // npm/ini only unquotes when the entire value is quoted. A comment after a
  // closing quote leaves the quote characters in the value, so reject that form.
  if value.starts_with(['\'', '"']) {
    let delimiter = value.as_bytes()[0] as char;
    return value.strip_prefix(delimiter).and_then(|v| v.strip_suffix(delimiter))
      .filter(|v| !v.contains('\\'))
      .map(str::to_owned)
      .ok_or_else(|| config_error("Unsupported quoted npm registry URL; use a literal unquoted URL before an inline comment"));
  }

  let mut escaped = false;
  let mut end = value.len();

  for (index, c) in value.char_indices() {
    if escaped {
      escaped = false;
      continue;
    }

    if c == '\\' {
      escaped = true;
      continue;
    }

    if matches!(c, '#' | ';') {
      end = index;
      break;
    }
  }

  Ok(value[..end].trim().replace(r"\#", "#").replace(r"\;", ";"))
}

fn discover_npmrc(content: &str, registries: &mut BTreeSet<String>) -> Result<(), AuthError> {
  for line in root_ini_lines(content) {
    if let Some((key, value)) = ini_entry(line)
      && (key == "registry" || (key.starts_with('@') && key.ends_with(":registry")))
    {
      add_registry(&ini_value(value)?, registries)?;
    }
  }

  Ok(())
}

fn discover_yaml(content: &str, registries: &mut BTreeSet<String>) -> Result<(), AuthError> {
  let Some(root) = yaml_root(content)? else {
    return Ok(());
  };

  if let Some(value) = root.get("registry") {
    add_registry(
      &value
        .as_scalar()
        .ok_or_else(|| config_error("registry must be a URL string"))?
        .as_string(),
      registries,
    )?;
  }

  if let Some(value) = root.get("registries") {
    let mapping = value
      .as_mapping()
      .ok_or_else(|| config_error("registries must be a mapping"))?;

    for (key, value) in mapping.iter() {
      let key = key
        .as_scalar()
        .ok_or_else(|| config_error("Registry key must be a string"))?
        .as_string();

      if key.starts_with("https://") {
        add_registry(&key, registries)?;
      } else {
        add_registry(
          &value
            .as_scalar()
            .ok_or_else(|| config_error("Scoped registries must contain URL strings"))?
            .as_string(),
          registries,
        )?;
      }
    }
  }

  Ok(())
}

fn check_yaml_auth(content: &str, registries: &BTreeSet<String>) -> Result<(), AuthError> {
  let Some(root) = yaml_root(content)? else {
    return Ok(());
  };

  if let Some(auth) = root.get("_auth") {
    let map = auth
      .as_mapping()
      .ok_or_else(|| config_error("pnpm _auth must be a mapping"))?;

    for (key, value) in map.iter() {
      let key = key
        .as_scalar()
        .ok_or_else(|| config_error("Auth registry must be a string"))?
        .as_string();
      let mut matched = BTreeSet::new();

      add_registry(&key, &mut matched)?;

      if matched
        .iter()
        .any(|key| registries.iter().any(|r| key.starts_with(r)))
      {
        let scopes = value
          .as_mapping()
          .ok_or_else(|| config_error("Auth scopes must be a mapping"))?;

        if scopes
          .iter()
          .any(|(key, _)| key.as_scalar().is_none_or(|s| s.as_string() != "@"))
        {
          return Err(config_error(
            "Scope-specific GAR credentials in global config.yaml override a registry helper; reconcile them manually first",
          ));
        }
      }
    }
  }

  Ok(())
}

fn check_conflicts(content: &str, registries: &BTreeSet<String>) -> Result<(), AuthError> {
  for line in root_ini_lines(content) {
    let Some((key, _)) = ini_entry(line) else {
      continue;
    };

    let Some((scope, field)) = key.rsplit_once(':') else {
      continue;
    };

    if ![
      "_authToken",
      "_auth",
      "_password",
      "username",
      "tokenHelper",
    ]
    .contains(&field)
    {
      continue;
    }

    let scope = format!("{}/", scope.trim_end_matches('/'));

    for registry in registries {
      if !registries.contains(&scope) && scope.starts_with(registry) {
        return Err(config_error(
          "More-specific GAR credentials or tokenHelper override the registry helper; reconcile the child-path or package-scope entry manually first",
        ));
      }
    }
  }

  Ok(())
}

fn resolve_helper(explicit: Option<&Path>) -> Result<String, AuthError> {
  let name = format!("node-auth-token{}", std::env::consts::EXE_SUFFIX);
  let path = if let Some(path) = explicit {
    path.to_path_buf()
  } else {
    let sibling = std::env::current_exe()?.with_file_name(&name);

    if sibling.is_file() {
      sibling
    } else {
      std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p.is_absolute())
        .map(|p| p.join(&name)).find(|p| p.is_file())
        .ok_or_else(|| config_error("node-auth-token is missing; install all node-auth binaries or pass --token-helper with its installed path"))?
    }
  };
  let path = path
    .canonicalize()
    .map_err(|_| config_error("node-auth-token executable does not exist"))?;

  if !path.is_file() {
    return Err(config_error("Token helper must be an executable file"));
  }

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;

    if path.metadata()?.permissions().mode() & 0o111 == 0 {
      return Err(config_error(
        "Token helper is not executable; fix its execute permissions",
      ));
    }
  }

  #[cfg(windows)]
  if !path
    .extension()
    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
  {
    return Err(config_error("Token helper must be a native .exe file"));
  }

  let value = path
    .to_str()
    .ok_or_else(|| config_error("Token helper path must be UTF-8"))?;
  let value = if cfg!(windows) {
    // canonicalize produces Windows verbatim paths, which pnpm/Node cannot spawn.
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
      format!("//{}", unc.replace('\\', "/"))
    } else {
      value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('\\', "/")
    }
  } else {
    value.into()
  };

  if value
    .chars()
    .any(|c| c.is_whitespace() || c.is_control() || "$%`\"'#;\\".contains(c))
  {
    return Err(config_error(
      "pnpm cannot safely represent this tokenHelper path; install node-auth-token in an absolute path without spaces, quotes, $, %, backticks, backslashes, # or ;",
    ));
  }

  Ok(value)
}

fn transform(
  content: &str,
  registries: &BTreeSet<String>,
  helper: &str,
) -> Result<String, AuthError> {
  let newline = if content.contains("\r\n") {
    "\r\n"
  } else {
    "\n"
  };

  let mut seen = BTreeSet::new();
  let mut offset = 0;
  let mut section_start = content.len();

  for line in content.split_inclusive('\n') {
    if line.trim_start().starts_with('[') {
      section_start = offset;
      break;
    }

    offset += line.len();
  }

  let (root, sections) = content.split_at(section_start);
  let mut output = String::new();

  for line in root.split_inclusive('\n') {
    if let Some((key, _)) = ini_entry(line)
      && let Some(registry) = key.strip_suffix(":tokenHelper")
    {
      // Canonicalize existing helper keys so variants without a trailing slash don't duplicate.
      if let Some(parsed) = (RegistryPolicy {
        allow_all_domains: false,
      })
      .parse_npm_key(registry)?
      {
        let registry = parsed.npm_key();

        if registries.contains(registry) {
          if seen.insert(registry.to_owned()) {
            let replacement = format!("{registry}:tokenHelper={helper}");

            if line.trim_end_matches(['\r', '\n']) == replacement {
              output.push_str(line);
            } else {
              output.push_str(&replacement);
              output.push_str(newline);
            }
          }

          continue;
        }
      }
    }

    output.push_str(line);
  }

  for registry in registries.difference(&seen) {
    if !output.is_empty() && !output.ends_with('\n') {
      output.push_str(newline);
    }

    output.push_str(&format!("{registry}:tokenHelper={helper}{newline}"));
  }

  output.push_str(sections);

  Ok(output)
}
