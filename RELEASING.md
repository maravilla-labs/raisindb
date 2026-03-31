# Releasing RaisinDB

## Server Binary Release (GitHub Releases)

The server binary is built for Linux x64, macOS x64/arm64, and Windows x64.

### Tag-driven release
```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

The workflow builds all platforms, generates `SHA256SUMS`, and creates a GitHub Release.

### Manual dispatch
Go to Actions > "Server cross-platform release" > Run workflow. Provide a tag or use auto-bump.

### Targets
| Platform | Target | Archive |
|----------|--------|---------|
| Linux x64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| macOS x64 | `x86_64-apple-darwin` | `.tar.gz` |
| macOS arm64 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `.zip` |

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
