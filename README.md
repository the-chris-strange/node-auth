# node-auth

[![GitHub](https://img.shields.io/badge/GitHub-the--chris--strange%2Fnode--auth-181717?logo=github)](https://github.com/the-chris-strange/node-auth)
[![Crates.io](https://img.shields.io/crates/v/node-auth.svg)](https://crates.io/crates/node-auth)
[![docs.rs](https://docs.rs/node-auth/badge.svg)](https://docs.rs/node-auth)
[![CI](https://github.com/the-chris-strange/node-auth/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/the-chris-strange/node-auth/actions/workflows/ci.yml)

A fast, lightweight, standalone Rust CLI and library that authenticates JavaScript package managers (**npm**, **yarn**, **pnpm**, and **Bun**) to private **Google Artifact Registry (GAR)** npm repositories using **Google Cloud Application Default Credentials (ADC)**.

It replicates and improves upon the functionality of Google's [`@google-cloud/artifact-registry-npm-tools`](https://github.com/GoogleCloudPlatform/artifact-registry-npm-tools), but **without requiring a JavaScript runtime or package manager to be installed or configured beforehand**.

---

## Why `node-auth`?

Google's official `google-artifactregistry-auth` tool is distributed as an npm package, which creates a chicken-and-egg problem: in minimal CI/CD containers, Docker build stages, or restricted developer workstations, you often need authentication *before* Node/npm is bootstrapped or when bootstrapping dependency caches. It introduces heavy Node.js runtime dependencies just to write an authentication token to `.npmrc`.

`node-auth` provides native binaries for **Windows**, **Linux**, and **macOS** and does not require a Node.js runtime.

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

This installs both `node-auth` and the compatibility command `artifactregistry-auth`.

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
mise use packslip:github.com/the-chris-strange/node-auth@0.1.0
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
# Binaries available in target/release/node-auth and target/release/artifactregistry-auth
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

- Reads registry configurations from `./.npmrc` (or `~/.npmrc` if no local file exists).
- If `./bunfig.toml` exists, reads Bun registry configurations from `install.registry` and `install.scopes`.
- Writes the authentication token to your user-level `~/.npmrc` (`%USERPROFILE%\.npmrc` on Windows) so secret tokens are never accidentally committed to Git.
- If `./.yarnrc.yml` exists, also updates Yarn's scope credentials.

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

1. Update `version` in `Cargo.toml`, update `Cargo.lock`, and merge the change.
2. Create and push the matching tag, for example `v0.2.0`.
3. Approve the protected `release` environment if repository rules require it.

The release workflow publishes the crate, creates native archives and checksums, creates the GitHub release, publishes a signed Packslip manifest with build provenance, and pushes package definitions for the [brew tap](https://github.com/the-chris-strange/homebrew-tap) and [scoop bucket](https://github.com/the-chris-strange/scoop-bucket).
