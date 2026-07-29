# Releasing RaisinDB

## Server Binary Release (GitHub Releases)

The server binary is built for Linux x64, macOS arm64, and Windows x64.

The release version comes from the **git tag** — the `build` job rewrites
`crates/raisin-server/Cargo.toml` from it before compiling. That file's `version`
is therefore not authoritative and is expected to lag behind released tags; don't
bump it by hand as part of a release.

### Tag-driven release
```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

The workflow builds all platforms, generates `SHA256SUMS`, and creates a GitHub Release.

### Manual dispatch
Go to Actions > "Server cross-platform release" > Run workflow. Provide a tag or use auto-bump.

### Linux-only (urgent server fix)
Windows is the long pole (~45 min for everything, ~30 for Linux alone). When only
the server binary matters — e.g. a deploy that pulls
`raisindb-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` — narrow the build:

```bash
gh workflow run release.yml -f tag=v0.1.0 -f targets=linux-only
```

**This only works via dispatch.** A tag *push* hard-codes `all`
(`github.event_name == 'push' && 'all'`), so don't push the tag if you want the
fast path — let the workflow create it.

Backfill the other platforms later on the same tag; publish uploads with
`--clobber`, so it tops up the existing release rather than conflicting:

```bash
gh workflow run release.yml -f tag=v0.1.0 -f targets=all
```

### Targets
| Platform | Target | Archive | Built when |
|----------|--------|---------|------------|
| Linux x64 | `x86_64-unknown-linux-gnu` | `.tar.gz` | always |
| macOS arm64 | `aarch64-apple-darwin` | `.tar.gz` | `targets=all` |
| Windows x64 | `x86_64-pc-windows-msvc` | `.zip` | `targets=all` |

### Verify
```bash
# Download and check
./raisindb --version
# Or via SHA256SUMS
sha256sum -c SHA256SUMS
```

## npm Packages (@raisindb/client, @raisindb/cli)

### Tag-driven publish
```bash
git tag -a npm-v0.1.0 -m "Publish npm packages v0.1.0"
git push origin npm-v0.1.0
```

### Manual dispatch
Go to Actions > "Publish npm packages" > Run workflow. Select which package to publish.

### Verify
```bash
npm view @raisindb/client version
npm view @raisindb/cli version
```

## User Installation

### Option 1: npm CLI (recommended for development)
```bash
npm install -g @raisindb/cli
raisindb server install   # downloads the server binary
raisindb server start     # starts the server
```

### Option 2: Direct binary download
Download from [GitHub Releases](https://github.com/maravilla-labs/raisindb/releases).

### Option 3: Build from source
```bash
cargo build --release -p raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/release/raisin-server
```

## Required Secrets

| Secret | Purpose |
|--------|---------|
| `NPM_TOKEN` | npm publish token for `@raisindb` scope |
| `GITHUB_TOKEN` | Auto-provided, used for GitHub Releases |

## Typical Release Flow

```bash
# 1. Release server binary
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0

# 2. Wait for CI to finish building all platforms

# 3. Publish npm packages (references the server release)
git tag -a npm-v0.1.0 -m "Publish npm v0.1.0"
git push origin npm-v0.1.0
```
