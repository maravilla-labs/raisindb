// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! External dependency setup for RaisinDB server.
//!
//! This module handles checking and setting up external dependencies like Tesseract OCR
//! during server startup. It uses a file-based state to persist setup decisions.

use anyhow::Result;
use raisin_deps::{
    DependencySetupState, DependencySetupUI, ExternalDependencyManager, SetupError,
    TesseractChecker,
};
use std::path::Path;

/// State file name for dependency setup.
const STATE_FILE: &str = "dependency_setup.json";

/// Run dependency setup during server startup.
///
/// This function:
/// 1. Loads persisted setup state from the data directory
/// 2. Runs interactive setup for any unconfigured dependencies (if TTY available)
/// 3. Updates global DEPENDENCY_FLAGS based on current availability
/// 4. Saves state back to disk
///
/// # Arguments
///
/// * `data_dir` - Path to the data directory where state is persisted
///
/// # Returns
///
/// Returns `Ok(())` on success, or exits if user chooses to exit during setup.
pub fn run_dependency_setup(data_dir: &str) -> Result<()> {
    let data_path = Path::new(data_dir);

    // Ensure data directory exists
    if !data_path.exists() {
        std::fs::create_dir_all(data_path)?;
    }

    let state_path = data_path.join(STATE_FILE);

    // Load existing state or create new
    let mut state = load_state(&state_path)?;

    // Create dependency manager with all checkers
    let manager = ExternalDependencyManager::new().register(TesseractChecker);

    // Check if we're running interactively
    let interactive = DependencySetupUI::is_interactive();

    // Run setup
    match manager.run_setup(&mut state, interactive) {
        Ok(result) => {
            // Log results
            if !result.installed.is_empty() {
                tracing::info!(
                    dependencies = ?result.installed,
                    "External dependencies available"
                );
            }

            if !result.skipped.is_empty() {
                tracing::warn!(
                    dependencies = ?result.skipped,
                    "External dependencies skipped - some features will be disabled"
                );
                DependencySetupUI::log_skipped_banner(
                    &result
                        .skipped
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>(),
                );
            }

            // Save state
            save_state(&state_path, &state)?;

            Ok(())
        }
        Err(SetupError::UserExit) => {
            tracing::info!("User exited dependency setup - shutting down");
            std::process::exit(0);
        }
        Err(e) => Err(anyhow::anyhow!("Dependency setup failed: {}", e)),
    }
}

/// Load dependency setup state from file.
fn load_state(path: &Path) -> Result<DependencySetupState> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let state: DependencySetupState = serde_json::from_str(&content).unwrap_or_default();
        Ok(state)
    } else {
        Ok(DependencySetupState::default())
    }
}

/// Save dependency setup state to file.
fn save_state(path: &Path, state: &DependencySetupState) -> Result<()> {
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Re-check a dependency and enable it if now available.
///
/// This is called from the admin API when user clicks "Enable" button.
pub fn try_enable_dependency(data_dir: &str, name: &str) -> Result<EnableResult> {
    let data_path = Path::new(data_dir);
    let state_path = data_path.join(STATE_FILE);

    // Load state
    let mut state = load_state(&state_path)?;

    // Create manager
    let manager = ExternalDependencyManager::new().register(TesseractChecker);

    // Try to enable
    match manager.try_enable(name, &mut state) {
        Ok(result) => {
            // Save state
            save_state(&state_path, &state)?;

            Ok(match result {
                raisin_deps::EnableResult::Enabled { version } => EnableResult::Enabled { version },
                raisin_deps::EnableResult::NotInstalled { instructions } => {
                    EnableResult::NotInstalled {
                        platform: instructions.platform.display_name().to_string(),
                        command: instructions.command,
                        needs_sudo: instructions.needs_sudo,
                        manual_url: instructions.manual_url,
                    }
                }
            })
        }
        Err(e) => Err(anyhow::anyhow!("Failed to enable dependency: {}", e)),
    }
}

/// Get current status of all dependencies.
pub fn get_dependency_status(data_dir: &str) -> Result<Vec<DependencyInfo>> {
    let data_path = Path::new(data_dir);
    let state_path = data_path.join(STATE_FILE);

    // Load state
    let state = load_state(&state_path)?;

    // Create manager
    let manager = ExternalDependencyManager::new().register(TesseractChecker);

    // Get status for all registered dependencies
    let results = manager.check_all();

    let infos = results
        .into_iter()
        .map(|r| {
            let skipped = state.is_skipped(&r.name);
            let status = if r.status.is_installed() {
                DependencyStatusInfo::Installed {
                    version: r.status.version().map(|s| s.to_string()),
                }
            } else if skipped {
                DependencyStatusInfo::Skipped
            } else {
                DependencyStatusInfo::NotAvailable
            };

            // Get install instructions
            let checker = manager.get_checker(&r.name);
            let instructions = checker.map(|c| {
                let inst = c.install_instructions();
                InstallInfo {
                    platform: inst.platform.display_name().to_string(),
                    command: inst.command,
                    needs_sudo: inst.needs_sudo,
                    manual_url: inst.manual_url,
                }
            });

            DependencyInfo {
                name: r.name,
                display_name: r.display_name,
                status,
                features_affected: r.features_affected,
                install_instructions: instructions,
            }
        })
        .collect();

    Ok(infos)
}

/// Result of trying to enable a dependency.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnableResult {
    /// Dependency is now enabled
    Enabled { version: String },
    /// Dependency is still not installed
    NotInstalled {
        platform: String,
        command: String,
        needs_sudo: bool,
        manual_url: Option<String>,
    },
}

/// Information about a dependency.
#[derive(Debug, serde::Serialize)]
pub struct DependencyInfo {
    pub name: String,
    pub display_name: String,
    pub status: DependencyStatusInfo,
    pub features_affected: Vec<String>,
    pub install_instructions: Option<InstallInfo>,
}

/// Status information for API.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DependencyStatusInfo {
    Installed { version: Option<String> },
    Skipped,
    NotAvailable,
}

/// Install instructions for API.
#[derive(Debug, serde::Serialize)]
pub struct InstallInfo {
    pub platform: String,
    pub command: String,
    pub needs_sudo: bool,
    pub manual_url: Option<String>,
}
