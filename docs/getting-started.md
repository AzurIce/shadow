# Getting Started

This guide walks through a complete Shadow workflow using Volcengine TOS.

## 1. Prerequisites

You need:

- Git
- a current Rust toolchain
- an existing Volcengine TOS bucket using the flat namespace model
- TOS credentials with permission to read, write, and inspect objects under the configured prefix

Install Shadow:

```bash
cargo install --git https://github.com/AzurIce/shadow
```

## 2. Initialize a Repository

Run the command anywhere inside an existing Git repository:

```bash
shadow init
```

Shadow discovers the Git root and creates `shadow.toml` plus the `.shadow/` runtime directories. It also appends an exact `# shadow` marker to the root `.gitignore` when one is not already present.

## 3. Configure TOS

Edit `shadow.toml`:

```toml
version = 1
name = "example-assets"

[backend]
type = "volcengine_tos"
endpoint = "https://tos-cn-beijing.volces.com"
region = "cn-beijing"
bucket = "example-shadow"
prefix = "shadow"
```

Configuration fields:

| Field | Meaning |
| --- | --- |
| `name` | A stable project namespace inside the bucket and prefix. |
| `endpoint` | The TOS API endpoint used by the SDK. |
| `region` | The TOS region used for request signing. |
| `bucket` | An existing bucket; Shadow does not create buckets. |
| `prefix` | An optional shared-bucket prefix. Use `""` for no prefix. |

Changing `name`, `bucket`, or `prefix` changes where Shadow looks for objects. Treat them as stable repository identity once refs have been published.

## 4. Configure Credentials

Shadow reads these environment variables:

```text
TOS_ACCESS_KEY
TOS_SECRET_KEY
TOS_SECURITY_TOKEN
```

`TOS_SECURITY_TOKEN` is optional and is only needed for temporary STS credentials.

You can place the values in a repository-root `.env` file:

```dotenv
TOS_ACCESS_KEY=...
TOS_SECRET_KEY=...
TOS_SECURITY_TOKEN=...
```

Existing process environment variables take precedence over `.env`. Always add `.env` to the ordinary Gitignore section and never commit credentials.

## 5. Declare Managed Files

Rules after `# shadow` in the root `.gitignore` define the Shadow-managed region:

```gitignore
.env
/target/

# shadow
/assets/**
/recordings/**/*.mp4
!/assets/keep-in-git.txt
```

The section uses Gitignore semantics. Files matched as ignored are managed by Shadow; negated rules remove files from the managed set.

Keep the marker exact and unique:

```text
# shadow
```

## 6. Inspect and Publish

Inspect the local state:

```bash
shadow status
```

Typical states are:

| State | Meaning |
| --- | --- |
| `Unpublished` | A managed worktree file exists without a ref. |
| `Published` | The worktree file matches its ref. |
| `Modified` | The worktree file and ref point to different content. |
| `Missing` | A ref exists but the worktree file does not. |
| `Orphaned` | A ref exists for a path no longer covered by Shadow rules. |

Publish all managed files:

```bash
shadow publish
```

Publishing performs the following operations:

1. imports a stable snapshot into the local cache;
2. computes SHA-256 and size;
3. detects a canonical HTTP media type;
4. checks whether the remote object already exists;
5. uploads missing content or repairs object metadata;
6. verifies the remote object;
7. atomically writes the ref.

Commit the refs and configuration:

```bash
git add .gitignore shadow.toml .shadow/refs
git commit -m "Track assets with Shadow"
```

## 7. Clone and Restore

After cloning the Git repository on another machine, configure credentials and run:

```bash
shadow status
shadow restore
```

Shadow downloads missing objects into `.shadow/tmp/`, validates SHA-256 and size, promotes them into the cache, and then materializes worktree files.

If a worktree file exists but differs from its ref, normal restore refuses to overwrite it. Replace modified files only when all refs are intentionally authoritative:

```bash
shadow restore --force
```

`--force` applies to the entire repository. Run `shadow status` first and review every modified path.

## 8. Status and Verification

`status` is an informational command. It hides healthy published files and groups only actionable states:

```bash
shadow status
shadow status --remote
```

`check` validates the entire repository and exits unsuccessfully when it finds an issue:

```bash
shadow check
shadow check --remote
```

Use `check --remote` in CI when credentials and network access are available. A missing local cache entry is allowed because cache contents can be restored; an existing cache object with invalid content is an error.

## 9. Work with Another Repository

All commands accept a global Git-style `-C` option:

```bash
shadow -C ../another-repository status
shadow -C ../another-repository publish
```

`-C` changes the working directory before Shadow discovers the Git root, loads `.env`, or reads `shadow.toml`.

## 10. Remove a Reference

Refs are ordinary Git-tracked files. Remove one with Git:

```bash
git rm .shadow/refs/assets/old-logo.png.ref
```

Update the Shadow section in `.gitignore` manually when the worktree path should no longer be managed. Remote object deletion and garbage collection are intentionally outside the current command set.
