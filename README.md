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
- **Web-ready metadata** — uploaded objects receive a detected HTTP `Content-Type` and long-lived immutable caching.

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

Objects land under `<prefix>/<name>/objects/sha256/…` inside the bucket—see
[where objects land](#where-objects-land) for how the pieces compose.

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
| `shadow gc [--delete] [--grace-days <days>]` | Compare reachable Git refs with remote inventory and remove unreferenced objects. |
| `shadow free --keep-at <ref> [--delete] [--grace-days <days>]` | Release objects not referenced at the given commits; refs elsewhere in history become dangling. |

All commands accept the global `-C <path>` option. `status` follows the shape of `git status`: healthy published files are hidden and only actionable groups are printed. `check` hashes existing cache objects, checks the entire repository, and is suitable for CI.

Refs are ordinary tracked files. Remove one explicitly with Git instead of a Shadow-specific command:

```bash
git rm .shadow/refs/assets/old-logo.png.ref
```

Preview remote garbage collection after refs have been removed from all reachable Git history:

```bash
shadow gc
```

The default 30-day grace period protects recently uploaded objects. After reviewing the candidates,
delete them explicitly:

```bash
shadow gc --delete
```

GC scans refs reachable from all local branches, tags, and remote-tracking refs. It refuses deletion
if any referenced object is missing remotely, ignores non-canonical keys, and re-scans Git immediately
before deleting in batches.

`gc` never deletes an object that any reachable ref mentions. Repositories that treat published
objects as re-derivable artifacts can go further and release objects by retention policy:

```bash
shadow free --keep-at origin/main
```

Only the refs in the given commits' trees count as live; the flag is repeatable, e.g. the deployed
commit plus its predecessor. Named after C's `free()`, the command may invalidate pointers by
design: refs elsewhere in history keep pointing at released objects and become dangling. Before
deleting, `free` reports how many ref versions across how many paths will dangle, keeps the same
30-day grace period for recently uploaded objects, and stays a dry run until `--delete` is passed:

```bash
shadow free --keep-at origin/main --delete
```

## Repository Model

Shadow separates what Git tracks from where object bodies live. Git records facts, Shadow stores
bodies, and refs are the bridge between the two:

```text
worktree file    assets/hero.png                             managed, Git-ignored
committed ref    .shadow/refs/assets/hero.png.ref            pinned by Git
local cache      .shadow/cache/objects/sha256/ab/cdef...     immutable, per checkout
remote object    <prefix>/<name>/objects/sha256/ab/cdef...   in the configured bucket
```

The committed ref is readable TOML and pins the content identity:

```toml
version = 1
oid = "sha256:abcdef0123456789..."
size = 1048576
```

### Where objects land

Every upload stores the object under a key derived only from the project identity and the digest:

```text
<prefix>/<name>/objects/sha256/<first 2 hex>/<remaining 62 hex>
```

| Segment | Comes from | Role |
| --- | --- | --- |
| `prefix` | `[backend] prefix` in `shadow.toml` | Lets one bucket serve several tools or projects. Optional; with `""` keys start at `<name>/`. |
| `name` | `name` in `shadow.toml` | The project namespace; `shadow init` defaults it to the repository directory name. |
| `objects/sha256/…` | the object content | A two-character fan-out directory followed by the remaining 62 digest characters. |

For example, with `prefix = "shadow"`, `name = "my-project"`, and a digest starting `abcd…`, the
full key inside the configured bucket is:

```text
shadow/my-project/objects/sha256/ab/cd...
```

Together, `prefix` and `name` form the project namespace: every remote read, write, and inventory
scan (`status --remote`, `gc`, `free`) happens inside it. The worktree path (`assets/hero.png`) and
its extension never reach the object key, so renaming a file does not re-upload its bytes, and
identical content is stored once per namespace. The local cache mirrors only the digest portion and
omits `prefix` and `name`, because `.shadow/` already belongs to a single checkout.

Treat `bucket`, `prefix`, and `name` as stable identity once objects are published: changing any of
them points Shadow at a namespace that is still empty, and existing objects would have to be
re-published. The `[backend] endpoint` is used for SDK requests and request signing only; the
hostname browsers see is a bucket, custom-domain, or CDN concern (see
[serving public assets](docs/public-assets.md)).

## Public Assets

Shadow sets `Content-Type` and `Cache-Control: max-age=31536000, immutable` during normal and multipart uploads. A public object can therefore be used by a browser even though its content-addressed URL has no filename extension, and it can be cached without routine revalidation.

Public access is controlled by the bucket policy, custom domain, or CDN—not by Shadow. Keep the bucket private unless anonymous object access is intentional.

See [serving public assets](docs/public-assets.md) for URL construction, TOS access policy considerations, and caching guidance.

## Safety Model

- `publish` writes a ref only after the remote object is confirmed.
- `restore` verifies size and SHA-256 before promoting a download into the cache.
- A modified worktree file requires `restore --force` before it can be replaced.
- Cache objects are treated as immutable and are never exposed as writable hard links.
- Credentials are read from environment variables and are never written into refs.
- Remote object keys are generated from validated project names and SHA-256 IDs.
- `free` intentionally creates dangling refs; it always reports the dangling ref count before `--delete`.

## Documentation

- [Documentation index](docs/README.md)
- [Getting started](docs/getting-started.md)
- [Serving public assets](docs/public-assets.md)
- [Core design](docs/design.md)
- [Volcengine TOS backend](docs/backends/volcengine-tos.md)

## Project Status

Shadow is an early-stage project. The current ref and configuration formats are versioned, but command-line behavior and backend interfaces may still evolve before a stable release.
