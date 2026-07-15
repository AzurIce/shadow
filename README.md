# Shadow

Shadow is an explicit large-file storage tool for Git repositories. Git commits small ref files under `.shadow/refs/`; file bodies remain in the worktree, are ignored by Git, and are stored in an object-storage backend.

The first backend is Volcengine TOS. Shadow uses only flat object-storage operations so additional S3-style backends can be added behind the same `BlobStore` trait.

## Managed Files

Edit the repository root `.gitignore`. Every rule after the `# shadow` marker belongs to Shadow:

```gitignore
/target/
.env

# shadow
/assets/**/*.png
/models/**/*.bin
```

## Commands

```text
shadow init
shadow status [--remote] [paths...]
shadow publish [paths...]
shadow restore [--force] [paths...]
shadow remove <paths...>
shadow verify [--remote]
```

- `status` compares worktree files, refs, cache, and optionally the remote.
- `publish` treats the worktree as authoritative, deduplicates by SHA-256, uploads missing blobs, and writes refs only after the remote object is available.
- `restore` treats refs as authoritative, downloads through the local cache, verifies SHA-256, and refuses to overwrite modified files unless `--force` is used.

## Configuration

`shadow init` creates `shadow.toml` at the Git repository root:

```toml
version = 1
name = "my-project"

[backend]
type = "volcengine_tos"
endpoint = "https://tos-cn-beijing.volces.com"
region = "cn-beijing"
bucket = "example-shadow"
prefix = "shadow"
```

TOS credentials are loaded from:

```text
TOS_ACCESS_KEY
TOS_SECRET_KEY
TOS_SECURITY_TOKEN
```

Shadow loads these variables from `.env` at the Git repository root. Variables already present in the process environment take precedence over values in `.env`.

See [the design documentation](docs/design.md) and [the TOS backend design](docs/backends/volcengine-tos.md) for the complete model.
