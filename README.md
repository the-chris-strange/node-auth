# node-auth

A fast, lightweight, standalone Rust CLI and library that authenticates Node package managers (**npm**, **yarn**, and **pnpm**) to private **Google Artifact Registry (GAR)** npm repositories using **Google Cloud Application Default Credentials (ADC)**.

It replicates and improves upon the functionality of Google's [`@google-cloud/artifact-registry-npm-tools`](https://github.com/GoogleCloudPlatform/artifact-registry-npm-tools), but **without requiring Node, npm, or pnpm to be installed or configured beforehand**.

---

## Why `node-auth`?

Google's official `google-artifactregistry-auth` tool is distributed as an npm package. This creates a chicken-and-egg problem:

- In minimal CI/CD containers, Docker build stages, or restricted developer workstations, you often need authentication *before* Node/npm is bootstrapped or when bootstrapping dependency caches.
- It introduces heavy Node.js runtime dependencies just to write an authentication token to `.npmrc`.

`node-auth` is a single native static binary that runs instantly on **Windows**, **Linux**, and **macOS** with zero external runtime dependencies.

---

## Platform Support

- **Windows**: Managed workstations, supports PowerShell and CMD (`%USERPROFILE%`, `%APPDATA%`, and `gcloud.cmd`).
- **Linux**: CI/CD pipelines (GitHub Actions, GitLab CI, Cloud Build), containers (Debian, Ubuntu, Alpine), and servers.
- **macOS**: Local development workstations (Apple Silicon & Intel).

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
   - If ADC is not configured, falls back to `gcloud auth print-access-token` (or `gcloud.cmd` on Windows) for developers logged in via `gcloud auth login`.
4. **Actionable Guidance**:
   - If credentials cannot be found, prints exact setup commands.

---

## Installation

### From Source

```bash
cargo install --path .
```

This installs both `node-auth` and the drop-in alias `google-artifactregistry-auth` to `~/.cargo/bin`.

### Building Release Binaries

```bash
cargo build --release
# Binaries available in target/release/node-auth and target/release/google-artifactregistry-auth
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
- Writes the authentication token to your user-level `~/.npmrc` (`%USERPROFILE%\.npmrc` on Windows) so secret tokens are never accidentally committed to Git.
- If `./.yarnrc.yml` exists, also updates Yarn Modern scope credentials.

### Writing to Local `.npmrc` (`--local-credential` / `-l`)

In CI/CD jobs or container builds, you often want credentials written directly to the project directory:

```bash
node-auth --local-credential
```

When running with `--local-credential`:

- It writes the token directly to `./.npmrc` (creating the file if it doesn't exist).
- **Git Safety Check**:
  - If the directory is a Git repository and `.npmrc` is **not** listed in `.gitignore`, it outputs a prominent warning:

    ```text
    ⚠️  WARNING: Writing credentials to local .npmrc, but '.npmrc' is not ignored in .gitignore!
       Make sure to add '.npmrc' to your .gitignore to avoid committing secret tokens to version control.
    ```

  - If the directory is **not** a Git repository, it reminds:

    ```text
    ⚠️  WARNING: Writing credentials to local .npmrc in a directory that is not a Git repository.
       Ensure this file is never committed or pushed to version control.
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
| `--token <TOKEN>` | `NODE_AUTH_TOKEN` | - | Explicit token to use instead of ADC/gcloud. |
| `--allow-all-domains` | - | `false` | Allow attaching token to any registry domain, not just `*-npm.pkg.dev`. |
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

### Yarn Modern (`.yarnrc.yml`)

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

---

## CI/CD Examples

### GitHub Actions (Workload Identity Federation)

```yaml
name: CI
on: [push]

jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write

    steps:
      - uses: actions/checkout@v4

      - name: Authenticate to Google Cloud
        uses: google-github-actions/auth@v2
        with:
          workload_identity_provider: 'projects/123456789/locations/global/workloadIdentityPools/my-pool/providers/my-provider'
          service_account: 'my-service-account@my-project.iam.gserviceaccount.com'

      # Authenticate npm to GAR before running npm/pnpm/yarn install
      - name: Authenticate Node to Artifact Registry
        run: |
          node-auth --local-credential

      - name: Install dependencies
        run: npm ci
```

### Google Cloud Build

```yaml
steps:
  - name: 'us-docker.pkg.dev/my-project/tools/node-auth'
    args: ['--local-credential']

  - name: 'node:20'
    entrypoint: 'npm'
    args: ['ci']
```

---

## License

Apache-2.0
