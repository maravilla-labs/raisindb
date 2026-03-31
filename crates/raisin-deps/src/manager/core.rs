// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Core dependency manager implementation.

use crate::checker::{DependencyChecker, DependencyResult, DependencyStatus};
use crate::flags::DEPENDENCY_FLAGS;
use crate::state::DependencySetupState;
use crate::ui::{DependencySetupUI, InstallAction};
use std::sync::Arc;

use super::types::{EnableResult, SetupError, SetupResult};

/// Manager for external system dependencies.
///
/// Provides a unified interface for checking multiple dependencies,
/// running interactive setup, and managing their availability state.
///
/// # Example
///
/// ```rust,ignore
/// use raisin_deps::{ExternalDependencyManager, TesseractChecker};
///
/// let manager = ExternalDependencyManager::new()
///     .register(TesseractChecker);
///
/// // Check all registered dependencies
/// let results = manager.check_all();
///
/// // Run interactive setup for missing dependencies
/// manager.run_setup(&mut state, is_interactive).await?;
/// ```
pub struct ExternalDependencyManager {
    checkers: Vec<Arc<dyn DependencyChecker>>,
}

impl Default for ExternalDependencyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalDependencyManager {
    /// Create a new dependency manager.
    pub fn new() -> Self {
        Self {
            checkers: Vec::new(),
        }
    }

    /// Register a dependency checker.
    pub fn register<C: DependencyChecker + 'static>(mut self, checker: C) -> Self {
        self.checkers.push(Arc::new(checker));
        self
    }

    /// Check all registered dependencies and return their status.
    pub fn check_all(&self) -> Vec<DependencyResult> {
        self.checkers
            .iter()
            .map(|c| DependencyResult::from_checker(c.as_ref()))
            .collect()
    }

    /// Check a specific dependency by name.
    pub fn check(&self, name: &str) -> Option<DependencyResult> {
        self.checkers
            .iter()
            .find(|c| c.name() == name)
            .map(|c| DependencyResult::from_checker(c.as_ref()))
    }

    /// Get a checker by name.
    pub fn get_checker(&self, name: &str) -> Option<&dyn DependencyChecker> {
        self.checkers
            .iter()
            .find(|c| c.name() == name)
            .map(|c| c.as_ref())
    }

    /// Run interactive setup for all registered dependencies.
    ///
    /// This should be called during server startup. It will:
    /// 1. Skip dependencies that are already set up (installed or skipped)
    /// 2. Check if dependencies are installed
    /// 3. Prompt the user to install missing optional dependencies
    /// 4. Update the state and global flags accordingly
    ///
    /// # Arguments
    ///
    /// * `state` - Mutable reference to the dependency setup state
    /// * `interactive` - Whether to show interactive prompts (false for non-TTY)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if all required dependencies are available,
    /// or an error if a required dependency is missing and user exits.
    pub fn run_setup(
        &self,
        state: &mut DependencySetupState,
        interactive: bool,
    ) -> Result<SetupResult, SetupError> {
        let mut result = SetupResult::default();

        // Show initial status
        if interactive {
            let results = self.check_all();
            DependencySetupUI::show_status(&results);
        }

        for checker in &self.checkers {
            self.setup_single_dependency(checker.as_ref(), state, interactive, &mut result)?;
        }

        Ok(result)
    }

    /// Set up a single dependency.
    fn setup_single_dependency(
        &self,
        checker: &dyn DependencyChecker,
        state: &mut DependencySetupState,
        interactive: bool,
        result: &mut SetupResult,
    ) -> Result<(), SetupError> {
        let name = checker.name();

        // Skip if already set up
        if state.is_setup_completed(name) {
            self.handle_already_setup(checker, state, result);
            return Ok(());
        }

        // Check current status
        let status = checker.check();

        match &status {
            DependencyStatus::Installed { version, .. } => {
                self.handle_installed(name, version, state, result, interactive);
            }

            DependencyStatus::NotInstalled
            | DependencyStatus::WrongVersion { .. }
            | DependencyStatus::Error(_) => {
                self.handle_not_installed(checker, state, interactive, result)?;
            }
        }

        Ok(())
    }

    /// Handle a dependency that was already set up in a previous session.
    fn handle_already_setup(
        &self,
        checker: &dyn DependencyChecker,
        state: &DependencySetupState,
        result: &mut SetupResult,
    ) {
        let name = checker.name();
        let status = checker.check();

        if status.is_installed() {
            DEPENDENCY_FLAGS.set_available(name);
            result.installed.push(name.to_string());
        } else if state.is_skipped(name) {
            DEPENDENCY_FLAGS.set_skipped(name);
            result.skipped.push(name.to_string());
        } else {
            // Was marked as installed but now missing
            DEPENDENCY_FLAGS.set_unavailable(name);
            result.unavailable.push(name.to_string());
        }
    }

    /// Handle an installed dependency.
    fn handle_installed(
        &self,
        name: &str,
        version: &str,
        state: &mut DependencySetupState,
        result: &mut SetupResult,
        interactive: bool,
    ) {
        state.mark_installed(name, Some(version.to_string()));
        DEPENDENCY_FLAGS.set_available(name);
        result.installed.push(name.to_string());

        if interactive {
            tracing::info!(
                dependency = name,
                version = version,
                "Dependency found and marked as installed"
            );
        }
    }

    /// Handle a dependency that is not installed.
    fn handle_not_installed(
        &self,
        checker: &dyn DependencyChecker,
        state: &mut DependencySetupState,
        interactive: bool,
        result: &mut SetupResult,
    ) -> Result<(), SetupError> {
        let name = checker.name();

        if !interactive {
            // Non-interactive mode: just skip
            state.mark_skipped(name);
            DEPENDENCY_FLAGS.set_skipped(name);
            result.skipped.push(name.to_string());

            tracing::warn!(
                dependency = name,
                "Dependency not available, running non-interactively - skipping"
            );
            return Ok(());
        }

        // Interactive: prompt user
        let action = DependencySetupUI::prompt_install(checker);

        match action {
            InstallAction::RunCommand => {
                self.run_installation(checker, state, result);
            }

            InstallAction::Skip => {
                state.mark_skipped(name);
                DEPENDENCY_FLAGS.set_skipped(name);
                result.skipped.push(name.to_string());

                tracing::info!(
                    dependency = name,
                    features = ?checker.features_affected(),
                    "User skipped dependency installation"
                );
            }

            InstallAction::Exit => {
                return Err(SetupError::UserExit);
            }
        }

        Ok(())
    }

    /// Run the installation command for a dependency.
    fn run_installation(
        &self,
        checker: &dyn DependencyChecker,
        state: &mut DependencySetupState,
        result: &mut SetupResult,
    ) {
        let name = checker.name();
        let instructions = checker.install_instructions();

        match DependencySetupUI::run_install_command(&instructions) {
            Ok(()) => {
                // Verify installation
                let new_status = checker.check();
                if let DependencyStatus::Installed { version, .. } = new_status {
                    state.mark_installed(name, Some(version.clone()));
                    DEPENDENCY_FLAGS.set_available(name);
                    result.installed.push(name.to_string());
                } else {
                    // Installation may have succeeded but check still fails
                    tracing::warn!(
                        dependency = name,
                        "Installation command succeeded but dependency check \
                         still fails. May need terminal restart."
                    );
                    state.mark_skipped(name);
                    DEPENDENCY_FLAGS.set_skipped(name);
                    result.skipped.push(name.to_string());
                }
            }
            Err(e) => {
                tracing::error!(
                    dependency = name,
                    error = %e,
                    "Failed to run install command"
                );
                // Continue to let user decide
                state.mark_skipped(name);
                DEPENDENCY_FLAGS.set_skipped(name);
                result.skipped.push(name.to_string());
            }
        }
    }

    /// Re-check a dependency and enable it if now available.
    ///
    /// This is called from the admin API when user clicks "Enable" button.
    pub fn try_enable(
        &self,
        name: &str,
        state: &mut DependencySetupState,
    ) -> Result<EnableResult, SetupError> {
        let checker = self
            .get_checker(name)
            .ok_or_else(|| SetupError::UnknownDependency(name.to_string()))?;

        let status = checker.check();

        match status {
            DependencyStatus::Installed { version, .. } => {
                state.mark_installed(name, Some(version.clone()));
                DEPENDENCY_FLAGS.set_available(name);

                Ok(EnableResult::Enabled { version })
            }

            DependencyStatus::NotInstalled
            | DependencyStatus::WrongVersion { .. }
            | DependencyStatus::Error(_) => Ok(EnableResult::NotInstalled {
                instructions: checker.install_instructions(),
            }),
        }
    }

    /// Get all registered checker names.
    pub fn registered_names(&self) -> Vec<&str> {
        self.checkers.iter().map(|c| c.name()).collect()
    }
}
