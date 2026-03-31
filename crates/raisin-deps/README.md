# raisin-deps

External dependency management for RaisinDB.

## Overview

This crate provides infrastructure for checking, installing, and managing external system dependencies like Tesseract OCR, FFmpeg, and other tools that RaisinDB may optionally use.

## Features

- **Dependency Checking** - Detect if tools are installed, get versions, find executables
- **Platform Detection** - macOS, Windows, Linux (Debian/Ubuntu, Fedora/RHEL, Arch, Alpine, openSUSE)
- **Interactive Setup** - CLI prompts with installation guidance per platform
- **State Persistence** - Remember setup decisions across restarts
- **Feature Flags** - Global flags to enable/disable features based on availability

## Usage

```rust
use raisin_deps::{ExternalDependencyManager, TesseractChecker};

// Create manager and register checkers
let manager = ExternalDependencyManager::new()
    .register(TesseractChecker);

// Check all dependencies
let results = manager.check_all();
for result in &results {
    println!("{}: {:?}", result.name, result.status);
}

// Run interactive setup (during server startup)
manager.run_setup(&mut state, is_interactive)?;
```

## Components

| Module | Description |
|--------|-------------|
| `checker.rs` | `DependencyChecker` trait and status types |
| `manager.rs` | `ExternalDependencyManager` for orchestrating checks |
| `platform.rs` | Platform and Linux distro detection |
| `state.rs` | `DependencySetupState` persistence |
| `flags.rs` | Global feature flags (`DEPENDENCY_FLAGS`) |
| `ui.rs` | Interactive CLI setup UI |
| `tools/` | Individual tool checkers (Tesseract, etc.) |

## Implementing a Checker

```rust
use raisin_deps::{DependencyChecker, DependencyStatus, InstallInstructions};

pub struct MyToolChecker;

impl DependencyChecker for MyToolChecker {
    fn name(&self) -> &str { "mytool" }
    fn display_name(&self) -> &str { "My Tool" }
    fn description(&self) -> &str { "Used for X feature" }

    fn check(&self) -> DependencyStatus {
        // Check if installed, return status
    }

    fn install_instructions(&self) -> InstallInstructions {
        // Return platform-specific install commands
    }

    fn is_required(&self) -> bool { false }
    fn features_affected(&self) -> Vec<&str> { vec!["ocr"] }
}
```

## Platform Support

| Platform | Package Manager | Detection |
|----------|-----------------|-----------|
| macOS | Homebrew, MacPorts | `target_os` |
| Windows | winget, Chocolatey, Scoop | `target_os` |
| Debian/Ubuntu | apt | `/etc/os-release` |
| Fedora/RHEL | dnf, yum | `/etc/os-release` |
| Arch | pacman | `/etc/os-release` |
| Alpine | apk | `/etc/os-release` |
| openSUSE | zypper | `/etc/os-release` |

## Dependency Status

```rust
pub enum DependencyStatus {
    Installed { version: String, path: PathBuf },
    NotInstalled,
    WrongVersion { found: String, required: String },
    Error(String),
}
```

## Built-in Checkers

| Checker | Tool | Features Affected |
|---------|------|-------------------|
| `TesseractChecker` | Tesseract OCR | PDF text extraction, image OCR |

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
