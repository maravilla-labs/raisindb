# Contributing to RaisinDB

Thank you for your interest in contributing to RaisinDB! This document provides guidelines for contributing to the project.

## Contributor License Agreement (CLA)

Before we can accept your contributions, you must agree to our [Contributor License Agreement](CLA.md). By submitting a pull request, you acknowledge that you have read and agree to the CLA.

## Getting Started

1. Fork the repository
2. Clone your fork
3. Create a new branch for your feature or fix
4. Make your changes
5. Run the test suite
6. Submit a pull request

## Development Setup

### Prerequisites

- Rust 1.89+ (stable)
- Node.js 20+ (for JS/TS packages)
- pnpm (for JS/TS package management)

### Building

```bash
# Build all Rust crates
cargo build --workspace

# Build the server with all features
cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"

# Build JS/TS packages
pnpm install
pnpm build
```

### Testing

```bash
# Run all workspace tests
cargo test --workspace

# Run a specific test
cargo test --package raisin-server --test cluster_social_feed_test -- --ignored --nocapture

# Quality checks
cargo fmt --workspace
cargo clippy --workspace
```

## Code Style

- Follow existing code conventions in the codebase
- Use `cargo fmt` for Rust formatting
- Use `cargo clippy` to catch common mistakes
- Keep files under 300 lines where practical
- Use `///` doc comments for public APIs
- Error handling: use `raisin-error` types with `thiserror` + `anyhow`

## Pull Request Process

1. Ensure your code compiles without warnings (`cargo clippy --workspace`)
2. Ensure all tests pass (`cargo test --workspace`)
3. Update documentation if you've changed public APIs
4. Write a clear PR description explaining the change and motivation
5. Link any related issues

## Reporting Issues

- Search [existing issues](https://github.com/maravilla-labs/raisindb/issues) before creating a new one
- Include reproduction steps, expected behavior, and actual behavior
- Include your Rust version and OS

## License

By contributing to RaisinDB, you agree that your contributions will be licensed under the [Business Source License 1.1](LICENSE), which converts to Apache License 2.0 after the change date.
