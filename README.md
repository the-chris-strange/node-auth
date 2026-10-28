# node-auth

[![GitHub](https://img.shields.io/badge/GitHub-the--chris--strange%2Fnode--auth-181717?logo=github)](https://github.com/the-chris-strange/node-auth)
[![Crates.io](https://img.shields.io/crates/v/node-auth.svg)](https://crates.io/crates/node-auth)
[![docs.rs](https://docs.rs/node-auth/badge.svg)](https://docs.rs/node-auth)
[![CI](https://github.com/the-chris-strange/node-auth/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/the-chris-strange/node-auth/actions/workflows/ci.yml)

A fast, lightweight, standalone Rust CLI and library that authenticates JavaScript package managers (**npm**, **yarn**, **pnpm**, and **Bun**) to private **Google Artifact Registry (GAR)** npm repositories using **Google Cloud Application Default Credentials (ADC)**.

It replicates and improves upon the functionality of Google's [`@google-cloud/artifact-registry-npm-tools`](https://github.com/GoogleCloudPlatform/artifact-registry-npm-tools), but without requiring a JavaScript runtime or package manager to be installed or configured beforehand.

---

## Why `node-auth`?

Google's official `google-artifactregistry-auth` tool is distributed as an npm package, which creates a chicken-and-egg problem: in minimal CI/CD containers, Docker build stages, or restricted developer workstations, you often need authentication *before* Node/npm is bootstrapped or when bootstrapping dependency caches. It introduces heavy Node.js runtime dependencies just to write an authentication token to `.npmrc`.

`node-auth` provides native binaries for all major operating systems and does not require a Node.js runtime.

---

## Platform Support

- **Windows**: Developer workstations, supports PowerShell and CMD (`%USERPROFILE%`, `%APPDATA%`, and `gcloud.cmd`).
- **Linux**: CI/CD pipelines (GitHub Actions, GitLab CI, Cloud Build), containers (Debian, Ubuntu, Alpine), and servers.
- **macOS**: Developer workstations (Apple Silicon & Intel).

---

## How Authentication Works

`node-auth` searches for credentials in the following order:

1. **Explicit Token**: If `--token <TOKEN>` (or `NODE_AUTH_TOKEN`) is provided, it is used immediately.
2. **Google Application Default Credentials (ADC)**:
   - `GOOGLE_APPLICATION_CREDENTIALS` environment variable (Service Account JSON, Workload Identity Federation).
   - Well-known user credentials created by `gcloud auth application-default login`:
     - Linux/macOS: `$HOME/.config/gcloud/application_default_credentials.json`
     - Windows: `%APPDATA%\gcloud\application_default_credentials.json`
   - GCE / Cloud Run / GKE / Cloud Build metadata service (`http://metadata.google.internal`).
3. **gcloud CLI Fallback**:
   - If ADC is not configured, falls back to `gcloud auth print-access-token` for developers logged in via `gcloud auth login`.
4. **Actionable Guidance**:
   - If credentials cannot be found, prints exact setup commands.

---

## Installation

### Cargo

```bash
cargo install node-auth --locked
```

This installs `node-auth`, the compatibility command `artifactregistry-auth`, and the token helper `node-auth-token`.

### Homebrew

Install the latest version with [Homebrew](https://brew.sh/) on macOS or Linux:

```bash
brew install the-chris-strange/homebrew-tap/node-auth
```

### Scoop

Install the latest version with [Scoop](https://scoop.sh/) on Windows:

```powershell
scoop bucket add https://github.com/the-chris-strange/scoop-bucket
scoop install node-auth
```

### mise

Install the latest signed release globally with [mise](https://mise.jdx.dev/) using the Packslip backend:

```bash
mise use -g packslip:github.com/the-chris-strange/node-auth@latest
```

To install and pin `node-auth` for a project, run this from the project directory:

```bash
mise use packslip:github.com/the-chris-strange/node-auth@0.2.0
```

### GitHub Releases

Prebuilt archives for Linux x86-64, macOS Intel and Apple Silicon, and Windows x86-64 are available on the repository's Releases page. Each release includes a `SHA256SUMS` file.

### From Source

```bash
cargo install --path .
```

This installs both `node-auth` and the drop-in alias `artifactregistry-auth` to `~/.cargo/bin`.

### Building Release Binaries

```bash
cargo build --release
# Binaries: target/release/{node-auth,artifactregistry-auth,node-auth-token}
```

### Generating API Documentation

You can generate and view the Rustdoc API documentation using `cargo`:

```bash
# Generate documentation without external dependencies
cargo docs

# Generate and open in your default browser
cargo doc-open
```

Documentation will be generated at `target/doc/node_auth/index.html`.

---

## CLI Usage

### Basic Usage

Run in your project root:

```bash
node-auth
```

By default:

- Reads registry configurations from `.npmrc` (or `~/.npmrc` if no local file exists).
- If `bunfig.toml` exists, reads Bun registry configurations from `install.registry` and `install.scopes`.
- Writes the authentication token to your user-level `~/.npmrc` (`%USERPROFILE%\.npmrc` on Windows) so secret tokens are never accidentally committed to Git.
- If `.yarnrc.yml` exists, also updates Yarn's scope credentials.

### Bun Environment File (`--bun-env`)

For a Bun-specific local workflow, write the token to `.env.local` and update eligible registries in the project `bunfig.toml` to reference it:

```bash
node-auth --bun-env
bun install
```

This changes a registry such as:

```toml
[install.scopes]
myorg = { url = "https://us-central1-npm.pkg.dev/my-project/my-repo/", username = "old", password = "$OLD_PASSWORD" }
```

to token authentication without storing the token in TOML:

```toml
[install.scopes]
myorg = { url = "https://us-central1-npm.pkg.dev/my-project/my-repo/", token = "$NODE_AUTH_TOKEN" }
```

The current token is stored as `NODE_AUTH_TOKEN` in `.env.local`, which Bun loads automatically. Google access tokens expire, so rerun `node-auth --bun-env` when credentials need refreshing. The tool creates new credential files with private permissions, preserves existing file permissions, and warns if `.env.local` is tracked or not ignored by Git. Add `.env.local` to `.gitignore`; never commit it.

### Writing to Local `.npmrc` (`--local-credential` / `-l`)

In CI/CD jobs or container builds, you often want credentials written directly to the project directory:

```bash
node-auth --local-credential
```

When running with `--local-credential`:

- It writes the token directly to `./.npmrc` (creating the file if it doesn't exist).
- **Git Safety Check**:
  - If Git reports `.npmrc` as tracked or not ignored, it warns:

    ```text
    Warning: Local .npmrc is tracked or not ignored by Git; it may expose credentials if committed.
    ```

  - If the directory is **not** a Git repository, it reminds:

    ```text
    Warning: Local .npmrc is outside a Git repository; keep credentials out of version control.
    ```

### Only Print the Access Token (`--print-token`)

If you just need the token for custom scripts or environment variables:

```bash
export NPM_TOKEN=$(node-auth --print-token)
```

---

### Registry Discovery and Caching

By default, node-auth detects npm, pnpm, or Yarn from `packageManager` in `package.json`, workspace configuration, or lockfiles, searching parent directories for workspace settings. It queries the installed manager for its effective registries, including scoped routes:

- npm: `npm config list --json`.
- pnpm: `pnpm config get registries --json`.
- Yarn Classic: `yarn config list --json` (the Yarn and npm configuration objects).
- Yarn 2+: `yarn config --json` and `yarn config get npmScopes --json`.

Successful query results determine which registries receive credentials. Explicit `--repo-config` or `--repo-config-yarn` sources retain file-based discovery. Missing package managers also retain file-based discovery. Bun continues to use its existing configuration-file discovery. Query failures stop the update without writing credentials; raw command output is never printed because it may contain secrets. Each command has a 15-second timeout.

Results are cached for **five minutes**, separately for each working directory and package manager. The cache contains validated registry URLs, Yarn scope routes, pnpm's version, and invalidation metadata; it contains no access tokens or raw configuration output. It lives under `$XDG_CACHE_HOME/node-auth` when that absolute path is set, otherwise `node-auth` in the operating system's user cache directory. Deleting that directory forces fresh queries.

Changes to standard project, ancestor, and user configuration files, any environment variable, or the manager executable invalidate the cache immediately. npm queries from workspace packages run at the workspace root, and expired root `.npmrc` credentials are moved to the user credential file. Credential writes performed by node-auth do not force another query on the next run. Invalid or expired entries are refreshed, and an unwritable cache does not prevent authentication. Configuration extensions outside the tracked files may take up to five minutes to be reflected. A registry override on a later install command is not visible to an earlier discovery query.

### Dynamic pnpm Authentication

With pnpm **11.13.0 or newer**, configure authentication once in each project or workspace whose GAR repositories you use:

```bash
gcloud auth application-default login
node-auth configure pnpm
pnpm install
```

Install all three executables (`node-auth`, `artifactregistry-auth`, and `node-auth-token`). Cargo, native release archives, Homebrew, Scoop, and Packslip include the helper. The command resolves the installed helper beside `node-auth`, then searches absolute directories on `PATH`. To select another installation:

```bash
node-auth configure pnpm --token-helper /absolute/path/to/node-auth-token
node-auth configure pnpm --repo-config /path/to/another.npmrc
node-auth configure pnpm --help
```

`node-auth-token` is an argument-free executable that shares the library's `--print-token` path. It prints only an access token and a newline, sends errors to stderr, exits nonzero on failure, and never modifies configuration files. It uses `NODE_AUTH_TOKEN` if set; otherwise it obtains Google ADC and retains the existing `gcloud auth print-access-token` fallback. For automatic refresh, leave `NODE_AUTH_TOKEN` unset. The helper also builds with `--no-default-features`, without Clap or the colored CLI presentation dependencies.

The configuration command does not authenticate. It uses pnpm's resolved registry query and the same cache described above, plus any additional `.npmrc` passed with `--repo-config`. If pnpm reports `undefined` for the registry query, it falls back to discovering literal HTTPS GAR registry URLs from:

- The workspace root `.npmrc`, or the nearest package root `.npmrc` when there is no workspace; otherwise the current directory's `.npmrc`.
- The effective user `.npmrc` (including `PNPM_CONFIG_NPMRC_AUTH_FILE`, `PNPM_CONFIG_USERCONFIG`, global `npmrcAuthFile`, and `NPM_CONFIG_USERCONFIG`).
- An additional `.npmrc` passed with `--repo-config`.
- `registry` and `registries` in `pnpm-workspace.yaml` and global `config.yaml`. Both scope-to-URL mappings and URL-keyed registry declarations are recognized.
- Registry definitions in the existing pnpm `auth.ini`.

Yarn and Bun files are not pnpm registry sources. Native queries let pnpm apply its own registry precedence and environment expansion. File-based fallback rejects unresolved environment-variable URLs and unsupported registry shapes. Configuration remains GAR-only; this command does not accept `--allow-all-domains` or the static credential flags.

Credentials are configured only in pnpm's trusted user-level `auth.ini` (registry discovery metadata is also cached):

| Platform | Location |
| --- | --- |
| Any platform with `XDG_CONFIG_HOME` | `$XDG_CONFIG_HOME/pnpm/auth.ini` |
| macOS | `~/Library/Preferences/pnpm/auth.ini` |
| Linux | `~/.config/pnpm/auth.ini` |
| Windows | `%LOCALAPPDATA%/pnpm/config/auth.ini` (normally `~/AppData/Local/pnpm/config/auth.ini`) |

Windows falls back to `~/.config/pnpm/auth.ini` when `LOCALAPPDATA` is unavailable, matching pnpm. `XDG_CONFIG_HOME` must be absolute. Destinations resolving inside the project, including through symlinks, are rejected.

For example, the command writes:

```ini
//us-central1-npm.pkg.dev/my-project/my-repo/:tokenHelper=/absolute/path/to/node-auth-token
```

It preserves comments, unrelated settings, line endings, and static credentials. It updates existing helper paths, collapses duplicate entries for the selected repositories, and leaves an already configured file untouched. Re-run the command after moving the helper or adding GAR repositories. File updates use the existing locking, atomic replacement, and permission-preservation code.

pnpm loads project `.npmrc` settings ahead of `auth.ini`, then falls back to the user `.npmrc`. A helper takes precedence over static credentials at the **same registry key**, so existing npm tokens can remain. More-specific child-path or package-scope credentials can bypass a registry-wide helper; the command stops with an error and leaves files unchanged when it finds these conflicts. Review those entries manually before retrying. Structured `pnpm_config__auth` environment overrides also require manual reconciliation. Project-level helpers are rejected; remove them from the project before configuring the trusted user file.

**Compatibility:** pnpm 11.13 fixed rejection of helpers in `auth.ini`. Older pnpm versions should use the existing static `node-auth` flow or be upgraded. The command never adds pnpm-only settings to shared `~/.npmrc`, where npm can warn about unsupported keys. Ordinary npm usage continues to use existing credentials; refresh npm's static GAR token with `node-auth` when needed. No npm wrapper is required or installed.

**Helper paths:** pnpm 11.13 splits helper commands on whitespace and rejects quotes and certain special characters. Install the helper in a path without spaces, quotes, `$`, `%`, backticks, `#`, or `;`. Windows paths are written with forward slashes. Paths that cannot be represented safely produce an actionable error rather than a broken helper entry.

**Lifecycle:** pnpm invokes the native helper when constructing registry authentication for a command; the helper obtains a currently valid Google token. Repeated pnpm commands therefore do not require a manual credential refresh. This is not HTTP 403 interception, and token refresh during a long-running pnpm process depends on pnpm's own lifecycle. pnpm 11.13 allows 60 seconds for the helper. ADC or gcloud must work noninteractively when pnpm invokes it. A 403 may mean insufficient IAM permissions, a wrong repository, or an organizational policy; obtaining another token does not grant additional permissions.

To verify, run `node-auth configure pnpm` twice (the second reports `Already configured`), inspect the helper path in `auth.ini`, then run `pnpm install` in a project with a GAR dependency. Running `node-auth-token` directly prints a sensitive token; avoid recording it in logs. To reverse setup, remove the relevant `//...:tokenHelper=...` entries from `auth.ini`. Existing npm credentials remain available, but may need refreshing with `node-auth`.

Upstream references: [pnpm authentication settings](https://pnpm.io/npmrc), [pnpm 11.13 trust fix](https://github.com/pnpm/pnpm/releases/tag/v11.13.0), and
[npm configuration and unsupported keys](https://docs.npmjs.com/files/npmrc/).

### Optional pnpm Integration Checks

Normal tests use isolated files, explicit fixture tokens, and package-manager query fixtures; they do not need Google credentials or installed package managers. To check the installed package managers without network access:

```bash
NODE_AUTH_TEST_PNPM=/absolute/path/to/pnpm \
  cargo test --test pnpm_test real_package_managers -- --ignored
```

Select the actual pnpm executable when using Corepack so the isolated test home does not cause a download. npm must also be installed. This check exercises helper execution, precedence over static credentials, offline failure behavior, and npm configuration loading without warnings.

For an opt-in live GAR metadata request (requires valid ADC or a gcloud session):

```bash
NODE_AUTH_TEST_PNPM=/absolute/path/to/pnpm \
NODE_AUTH_GAR_REGISTRY=https://us-central1-npm.pkg.dev/my-project/my-repo/ \
NODE_AUTH_GAR_PACKAGE=@my-scope/my-package \
  cargo test --test pnpm_test live_gar -- --ignored
```

The live check reads your real Google credentials while keeping package-manager configuration in temporary directories. It is never required in CI.

## CLI Options

| Flag | Environment Variable | Default | Description |
| --- | --- | --- | --- |
| `-l`, `--local-credential` | - | `false` | Force writing credentials to `./.npmrc`. Checks `.gitignore` for safety. |
| `--repo-config <PATH>` | `NODE_AUTH_REPO_CONFIG` | `./.npmrc` or `~/.npmrc` | Path to `.npmrc` to read registry configs from. |
| `--credential-config <PATH>` | `NODE_AUTH_CREDENTIAL_CONFIG` | `~/.npmrc` | Path to `.npmrc` to write credentials to. |
| `--repo-config-yarn <PATH>` | - | `./.yarnrc.yml` or `~/.yarnrc.yml` | Path to `.yarnrc.yml` to read scope configurations from. |
| `--credential-config-yarn <PATH>` | - | `~/.yarnrc.yml` | Path to `.yarnrc.yml` to write credentials to. |
| `--yarn <BOOL>` | - | Auto-detect | Explicitly enable (`--yarn true`) or disable (`--yarn false`) Yarn updating. |
| `--repo-config-bun <PATH>` | - | `./bunfig.toml` | Path to `bunfig.toml` used to discover Bun registries. |
| `--bun <BOOL>` | - | Auto-detect | Explicitly enable (`--bun true`) or disable (`--bun false`) Bun discovery. |
| `--bun-env` | - | `false` | Update project `bunfig.toml` to use `$NODE_AUTH_TOKEN` and store the token in `.env.local`. Conflicts with `--local-credential`. |
| `--token <TOKEN>` | `NODE_AUTH_TOKEN` | - | Explicit token to use instead of ADC/gcloud. |
| `--allow-all-domains` | - | `false` | Allow attaching the token to other HTTPS registry domains in npm, Yarn, and Bun configuration. |
| `-v`, `--verbose` | - | `false` | Enable verbose logging output. |
| `--print-token` | - | `false` | Output access token to stdout without modifying config files. |
| `-h`, `--help` | - | - | Print help information. |

---

## Configuration Formats Supported

### NPM (`.npmrc`)

Supports standard scoped or unscoped Artifact Registry definitions:

```ini
@my-org:registry=https://us-central1-npm.pkg.dev/my-project/my-repo/
```

`node-auth` automatically generates or updates the credential line in the target `.npmrc`:

```ini
//us-central1-npm.pkg.dev/my-project/my-repo/:_authToken="ya29.a0AfH6..."
```

- Replaces legacy basic auth passwords (`:_password` and `username=oauth2accesstoken`).
- Strips sensitive `_authToken` entries out of project-level `.npmrc` when writing to user `~/.npmrc`.
- Preserves all other configs, comments, and other registries (e.g. `registry.npmjs.org`).

### Bun (`bunfig.toml` and `.npmrc`)

[Bun reads registry and authentication settings from `.npmrc`](https://bun.sh/docs/pm/npmrc), including scoped registry definitions and path-specific `_authToken` entries. The same `.npmrc` configuration generated by `node-auth` therefore authenticates `bun install`, `bun add`, and other Bun package-manager commands:

```bash
node-auth
bun install
```

`node-auth` also discovers default and scoped registries directly from project `bunfig.toml` files. By default, it leaves `bunfig.toml` unchanged and writes path-scoped credentials to the user `.npmrc`; Bun matches those credentials by registry host and path. This avoids putting secrets in a tracked project file and avoids project Bun configuration overriding user-level Bun settings.

Use `--bun-env` only when the project should manage Bun authentication through `.env.local`. It is intentionally Bun-specific: npm, Yarn, and pnpm do not share Bun's automatic environment-file behavior.

### Yarn (`.yarnrc.yml`)

If `.yarnrc.yml` is used:

```yaml
npmScopes:
  my-org:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-project/my-repo"
```

`node-auth` adds or updates:

```yaml
npmScopes:
  my-org:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-project/my-repo"
    npmAlwaysAuth: true
    npmAuthToken: "ya29.a0AfH6..."
```

Registry URLs must use HTTPS and may not contain embedded credentials, queries, or fragments. By default, only Artifact Registry hosts ending in `-npm.pkg.dev` receive a token; other Yarn scopes are left unchanged. `--allow-all-domains` explicitly permits other HTTPS hosts. Malformed Yarn YAML causes an error without replacing either rc file.

## File Updates and Library Use

Before reading, paths are resolved to absolute paths, including symlink targets. The tool holds a process lock for each affected parent directory during a run, stages all changes in temporary files beside their destinations, and replaces each file atomically. Unix uses an advisory lock on the existing directory; Windows uses a named kernel mutex. New credential files are created with owner-only permissions (`0600`) on Unix and a protected, current-user-only DACL on Windows. Existing Unix mode bits and Windows DACLs are preserved during replacement. The tool does not audit or repair permissions on files created by other applications.

The process locks do not create lock files. Replacing multiple rc files is not one filesystem-wide atomic operation; interruption between replacements can leave a partial update.

The `cli` Cargo feature is enabled by default. It includes both executables (`node-auth` and `artifactregistry-auth`), the CLI modules, and their argument parsing, terminal color, and runtime dependencies. For library-only use, disable default features:

```toml
[dependencies]
node-auth = { version = "0.1.0", default-features = false }
```

For synchronous applications, `run_blocking(&Options)` creates and manages a temporary Tokio runtime. No Tokio dependency or runtime configuration is needed in your application:

```rust
use node_auth::{run_blocking, Options};

fn main() -> Result<(), node_auth::AuthError> {
    let outcome = run_blocking(&Options::default())?;
    // Handle the returned token or updated configuration paths.
    Ok(())
}
```

Each call creates a current-thread runtime with networking and timers enabled and drops it before returning. The helper returns errors and outcomes without initializing logging or printing output.

Async applications should call `run(&Options).await` within their Tokio runtime. Calling `run_blocking` inside an active Tokio runtime returns an error; use `run().await` instead. This guard also rejects Tokio blocking-pool threads.

Tokio is a library dependency for the blocking helper and Google authentication. The `cli` feature additionally enables Tokio's macros and multithreaded runtime.

The Rust library returns `Result<RunOutcome, AuthError>` from `run(&Options)`. The outcome contains either a token for `print_token` or the paths updated and the local Git safety status.

## Maintainer Releases

CI runs formatting, lint, packaging, and tests on every pull request and push to `main`. Tests run on current GitHub-hosted Linux, macOS, and Windows runners.

To publish a release:

1. Open a PR updating `version` in `Cargo.toml` and the matching entry in `Cargo.lock`.
2. Merge the version-bump PR into `main`. GitHub Actions creates the matching tag (for example, `v0.2.0`) at the PR's merge commit and starts the release workflow. PRs without a crate version change are skipped. An existing tag pointing to another commit causes the automation to fail rather than move the tag.
3. Approve the protected `release` environment if repository rules require it.

The release workflow packages all three executables, publishes the crate, creates native archives and checksums, creates the GitHub release, publishes a signed Packslip manifest with build provenance, and pushes package definitions for the [brew tap](https://github.com/the-chris-strange/homebrew-tap) and [scoop bucket](https://github.com/the-chris-strange/scoop-bucket).

Package publishing uses `Justintime50/homebrew-releaser@v4` for Homebrew and `xberg-io/actions/publish-scoop-manifests@v1` for Scoop. Set `PACKAGE_REPOSITORY_TOKEN` as a repository secret or a secret in the `release` environment, with Contents read/write access to both package repositories and read access to this repository. The default destinations are `the-chris-strange/homebrew-tap` and `the-chris-strange/scoop-bucket`; override them with `HOMEBREW_TAP_REPOSITORY` and `SCOOP_BUCKET_REPOSITORY` variables at repository or `release` environment scope. Initialize each package repository with a commit (for example, a README) before the first release; empty repositories cannot be checked out. The workflow creates the Scoop `bucket/` directory.

The publishing jobs run after the GitHub release succeeds and report missing credentials as an error. Homebrew Releaser requires the latest stable release. The workflow adds its required archive names alongside the existing Rust-target archives and disables its checksum upload to support immutable releases.
