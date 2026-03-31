// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! CLI interactions for dependency setup.
//!
//! Uses the `inquire` crate to provide a professional CLI experience
//! similar to raisindb-cli for guiding users through dependency installation.

use crate::checker::{DependencyChecker, DependencyResult, DependencyStatus, InstallInstructions};
use colored::Colorize;
use inquire::Select;
use std::process::{Command, Stdio};

/// User action choice for dependency installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    /// Run the install command automatically
    RunCommand,
    /// Skip this dependency (features will be disabled)
    Skip,
    /// Exit setup to install manually
    Exit,
}

/// Interactive UI for dependency setup.
pub struct DependencySetupUI;

impl DependencySetupUI {
    /// Display the status of all dependencies.
    pub fn show_status(deps: &[DependencyResult]) {
        println!();
        println!("{}", "External Dependencies".bold());
        println!("{}", "─".repeat(60));

        for dep in deps {
            let status_icon = Self::format_status_line(dep);
            println!("{}", status_icon);

            // Show affected features if not installed
            if !dep.status.is_installed() && !dep.features_affected.is_empty() {
                let features = dep.features_affected.join(", ");
                println!("    {} {}", "↳".dimmed(), features.dimmed());
            }
        }

        println!();
    }

    /// Format a single status line for a dependency.
    fn format_status_line(dep: &DependencyResult) -> String {
        match &dep.status {
            DependencyStatus::Installed { version, .. } => {
                format!(
                    "  {} {} {}",
                    "✓".green(),
                    dep.display_name,
                    format!("({})", version).dimmed()
                )
            }
            DependencyStatus::NotInstalled => {
                format!(
                    "  {} {} {}",
                    "✗".red(),
                    dep.display_name,
                    "(not installed)".dimmed()
                )
            }
            DependencyStatus::WrongVersion { found, required } => {
                format!(
                    "  {} {} {} {} {}",
                    "!".yellow(),
                    dep.display_name,
                    format!("(v{})", found).dimmed(),
                    "requires".dimmed(),
                    format!("v{}", required).dimmed()
                )
            }
            DependencyStatus::Error(e) => {
                format!(
                    "  {} {} {}",
                    "?".yellow(),
                    dep.display_name,
                    format!("(error: {})", e).dimmed()
                )
            }
        }
    }

    /// Prompt user to install a missing dependency.
    pub fn prompt_install(dep: &dyn DependencyChecker) -> InstallAction {
        let instructions = dep.install_instructions();

        Self::print_dependency_info(dep, &instructions);

        let options = Self::build_options(&instructions);

        let answer = Select::new("How would you like to proceed?", options)
            .with_help_message("Use ↑↓ to navigate, Enter to select")
            .prompt();

        match answer {
            Ok(choice) => {
                if choice.starts_with("Run") {
                    InstallAction::RunCommand
                } else if choice.starts_with("Skip") {
                    InstallAction::Skip
                } else {
                    InstallAction::Exit
                }
            }
            Err(_) => {
                // User pressed Ctrl+C or Escape
                InstallAction::Exit
            }
        }
    }

    /// Print information about a dependency.
    fn print_dependency_info(dep: &dyn DependencyChecker, instructions: &InstallInstructions) {
        println!();
        println!(
            "{} {} is not installed.",
            "→".yellow(),
            dep.display_name().yellow().bold()
        );

        // Show description
        println!("  {}", dep.description().dimmed());

        // Show affected features
        let features = dep.features_affected();
        if !features.is_empty() {
            println!(
                "  {} {}",
                "Features affected:".dimmed(),
                features.join(", ").dimmed()
            );
        }

        // Show install command
        println!();
        println!("  {}", "Install command:".dimmed());
        println!("    {}", instructions.command.cyan());

        if instructions.needs_sudo {
            println!(
                "    {}",
                "(requires administrator privileges)".dimmed().italic()
            );
        }

        // Show post-install instructions if any
        if let Some(post) = &instructions.post_install {
            println!();
            println!("  {}", "After installation:".dimmed());
            println!("    {}", post.dimmed());
        }

        // Show manual URL if available
        if let Some(url) = &instructions.manual_url {
            println!();
            println!("  {}", "Manual installation:".dimmed());
            println!("    {}", url.blue().underline());
        }

        println!();
    }

    /// Build options based on whether command needs sudo.
    fn build_options(instructions: &InstallInstructions) -> Vec<&'static str> {
        if instructions.needs_sudo {
            vec![
                "Run install command (will prompt for password)",
                "Skip (features will be disabled)",
                "Exit and install manually",
            ]
        } else {
            vec![
                "Run install command",
                "Skip (features will be disabled)",
                "Exit and install manually",
            ]
        }
    }

    /// Run an install command with proper shell handling.
    pub fn run_install_command(instructions: &InstallInstructions) -> Result<(), std::io::Error> {
        let cmd = if instructions.needs_sudo {
            format!("sudo {}", instructions.command)
        } else {
            instructions.command.clone()
        };

        println!();
        println!("{}", "Running:".dimmed());
        println!("  {}", cmd.cyan());
        println!();

        // Determine shell based on platform
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // Run with inherited stdin/stdout/stderr for interactive commands
        let status = Command::new(shell)
            .arg(shell_arg)
            .arg(&cmd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        if status.success() {
            Self::print_success(instructions);
            Ok(())
        } else {
            Self::print_failure(&status);
            Err(std::io::Error::other(format!(
                "Command exited with code: {:?}",
                status.code().unwrap_or(-1)
            )))
        }
    }

    /// Print success message after installation.
    fn print_success(instructions: &InstallInstructions) {
        println!();
        println!("{} Installation complete!", "✓".green());

        // Show post-install message if any
        if let Some(post) = &instructions.post_install {
            println!();
            println!("{}", "Note:".yellow());
            println!("  {}", post);
        }
    }

    /// Print failure message after installation attempt.
    fn print_failure(status: &std::process::ExitStatus) {
        println!();
        println!(
            "{} Installation may have failed (exit code: {:?})",
            "!".yellow(),
            status.code()
        );
        println!("  {}", "You may need to run the command manually.".dimmed());
    }

    /// Display a banner for skipped dependencies (used in logs).
    pub fn log_skipped_banner(deps: &[&str]) {
        if deps.is_empty() {
            return;
        }

        let deps_list = deps.join(", ");
        tracing::warn!(
            dependencies = %deps_list,
            "Some external dependencies are not installed. \
             Related features will be disabled. \
             Enable them in Admin Console → Settings → AI Models"
        );
    }

    /// Check if the terminal supports interactive prompts.
    pub fn is_interactive() -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal()
    }
}
