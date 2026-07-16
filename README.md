# Shadow

Shadow is an explicit, content-addressed large-file storage tool for Git repositories.

Git tracks small TOML reference files under `.shadow/refs/`, while the file bodies stay in the worktree, remain ignored by Git, and are stored in object storage. Shadow never installs clean/smudge filters and never intercepts ordinary Git commands: publishing and restoring files are always explicit operations.

Volcengine TOS is the first supported backend. The storage layer is hidden behind a small `BlobStore` interface so S3-compatible backends can be added without changing the repository workflow.

## Why Shadow?

- **Explicit workflow** — `status`, `publish`, and `restore` make data movement visible.
- **Content-addressed storage** — SHA-256 object IDs provide deduplication and integrity checks.
- **Git-friendly references** — small, readable TOML refs are committed instead of large file bodies.
- **Local cache** — uploads and downloads pass through an immutable verified cache.
- **Safe restore** — modified worktree files are never overwritten without `--force`.
- **Backend-neutral core** — repository logic does not depend on TOS-specific types or features.
- **Web-ready metadata** — uploaded objects receive a detected HTTP `Content-Type`.

## Installation

Shadow currently requires a Rust toolchain and can be installed directly from GitHub:

```bash
cargo install --git https://github.com/AzurIce/shadow
```

To build a local checkout instead:

```bash
cargo build --release
```

The executable is written to `target/release/shadow` (`shadow.exe` on Windows).

## Quick Start

Initialize Shadow inside an existing Git repository:

```bash
shadow init
```

This creates:

```text
shadow.toml
.shadow/
├── .gitignore
├── refs/
├── cache/
└── tmp/
```

Configure a Volcengine TOS bucket in `shadow.toml`:

```toml
version = 1
name = "my-project"

[backend]
type = "volcengine_tos"
endpoint = "https://tos-cn-beijing.volces.com"
region = "cn-beijing"
bucket = "my-shadow-bucket"
prefix = "shadow"
```

Provide credentials through the process environment or a repository-root `.env` file:

```dotenv
TOS_ACCESS_KEY=your-access-key
TOS_SECRET_KEY=your-secret-key
# TOS_SECURITY_TOKEN=your-temporary-security-token
```

Keep `.env` outside the Shadow-managed section and ignored by Git.

Declare managed files after the exact `# shadow` marker in the root `.gitignore`:

```gitignore
/target/
.env

# shadow
/assets/**/*.png
/videos/**/*.mp4
/models/**/*.bin
```

Everything after the marker is interpreted as the Shadow rule section, including Gitignore negation rules.

Inspect, publish, and commit the generated refs:

```bash
shadow status
shadow publish
git add .gitignore shadow.toml .shadow/refs
git commit -m "Track large assets with Shadow"
```

After cloning the repository elsewhere, restore missing worktree files with:

```bash
shadow restore
```

See the [getting-started guide](docs/getting-started.md) for the complete workflow.

Run any command against another repository without changing the shell directory:

```bash
shadow -C ../another-repository status
```

## Commands

| Command | Purpose |
| --- | --- |
| `shadow init` | Initialize Shadow in the current Git repository. |
| `shadow status [--remote]` | Show unpublished, missing, orphaned, and optionally remote issues. |
| `shadow publish` | Treat the worktree as authoritative, upload objects, and write refs. |
| `shadow restore [--force]` | Treat refs as authoritative and materialize files through the cache. |
| `shadow check [--remote]` | Validate repository invariants and return a failing exit code on issues. |

All commands accept the global `-C <path>` option. `status` follows the shape of `git status`: healthy published files are hidden and only actionable groups are printed. `check` hashes existing cache objects, checks the entire repository, and is suitable for CI.

Refs are ordinary tracked files. Remove one explicitly with Git instead of a Shadow-specific command:

```bash
git rm .shadow/refs/assets/old-logo.png.ref
```

## Repository Model

For a managed file such as `assets/hero.png`, Git stores a ref at:

```text
.shadow/refs/assets/hero.png.ref
```

The ref is readable TOML:

```toml
version = 1
oid = "sha256:abcdef0123456789..."
size = 1048576
content_type = "image/png"
```

Object bodies are addressed only by their digest:

```text
Local cache:
.shadow/cache/objects/sha256/ab/cdef...

Remote object key:
<prefix>/<name>/objects/sha256/ab/cdef...
```

Original paths and extensions are deliberately excluded from object keys. Renaming a file does not upload its bytes again, and identical content is stored once within a project namespace.

## Public Assets

Shadow sets `Content-Type` during normal and multipart uploads. A public object can therefore be used by a browser even though its content-addressed URL has no filename extension.

Public access is controlled by the bucket policy, custom domain, or CDN—not by Shadow. Keep the bucket private unless anonymous object access is intentional.

See [serving public assets](docs/public-assets.md) for URL construction, TOS access policy considerations, and caching guidance.

## Safety Model

- `publish` writes a ref only after the remote object is confirmed.
- `restore` verifies size and SHA-256 before promoting a download into the cache.
- A modified worktree file requires `restore --force` before it can be replaced.
- Cache objects are treated as immutable and are never exposed as writable hard links.
- Credentials are read from environment variables and are never written into refs.
- Remote object keys are generated from validated project names and SHA-256 IDs.

## Documentation

- [Documentation index](docs/README.md)
- [Getting started](docs/getting-started.md)
- [Serving public assets](docs/public-assets.md)
- [Core design](docs/design.md)
- [Volcengine TOS backend](docs/backends/volcengine-tos.md)

## Project Status

Shadow is an early-stage project. The current ref and configuration formats are versioned, but command-line behavior and backend interfaces may still evolve before a stable release.
