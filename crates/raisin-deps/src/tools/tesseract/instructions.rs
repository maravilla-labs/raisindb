// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Platform-specific installation instructions for Tesseract.

use crate::checker::InstallInstructions;
use crate::platform::{command_exists, LinuxDistro, Platform};

/// Build installation instructions for the current platform.
pub fn build_install_instructions() -> InstallInstructions {
    let platform = Platform::detect();

    match platform {
        Platform::MacOS => build_macos_instructions(platform),
        Platform::Linux(distro) => build_linux_instructions(platform, distro),
        Platform::Windows => build_windows_instructions(platform),
        Platform::Unknown => build_unknown_instructions(platform),
    }
}

/// Build macOS-specific installation instructions.
fn build_macos_instructions(platform: Platform) -> InstallInstructions {
    // Check if brew is available
    if command_exists("brew") {
        InstallInstructions {
            platform,
            package_manager: Some("brew".to_string()),
            command: "brew install tesseract tesseract-lang".to_string(),
            needs_sudo: false,
            post_install: None,
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        }
    } else if command_exists("port") {
        InstallInstructions {
            platform,
            package_manager: Some("port".to_string()),
            command: "port install tesseract tesseract-eng".to_string(),
            needs_sudo: true,
            post_install: None,
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        }
    } else {
        InstallInstructions {
            platform,
            package_manager: None,
            command: "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\" && brew install tesseract tesseract-lang".to_string(),
            needs_sudo: false,
            post_install: Some("Restart your terminal after Homebrew installation".to_string()),
            manual_url: Some("https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string()),
            provides: vec!["PDF OCR".to_string(), "Scanned document text extraction".to_string()],
        }
    }
}

/// Build Linux-specific installation instructions.
fn build_linux_instructions(platform: Platform, distro: LinuxDistro) -> InstallInstructions {
    match distro {
        LinuxDistro::Debian => InstallInstructions {
            platform,
            package_manager: Some("apt".to_string()),
            command: "apt install -y tesseract-ocr tesseract-ocr-eng".to_string(),
            needs_sudo: true,
            post_install: Some(
                "Install additional languages with: apt install tesseract-ocr-<lang>".to_string(),
            ),
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },

        LinuxDistro::Fedora => InstallInstructions {
            platform,
            package_manager: Some("dnf".to_string()),
            command: "dnf install -y tesseract tesseract-langpack-eng".to_string(),
            needs_sudo: true,
            post_install: Some(
                "Install additional languages with: dnf install tesseract-langpack-<lang>"
                    .to_string(),
            ),
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },

        LinuxDistro::Arch => InstallInstructions {
            platform,
            package_manager: Some("pacman".to_string()),
            command: "pacman -S --noconfirm tesseract tesseract-data-eng".to_string(),
            needs_sudo: true,
            post_install: Some(
                "Install additional languages with: pacman -S tesseract-data-<lang>".to_string(),
            ),
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },

        LinuxDistro::Alpine => InstallInstructions {
            platform,
            package_manager: Some("apk".to_string()),
            command: "apk add tesseract-ocr tesseract-ocr-data-eng".to_string(),
            needs_sudo: true,
            post_install: None,
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },

        LinuxDistro::OpenSuse => InstallInstructions {
            platform,
            package_manager: Some("zypper".to_string()),
            command: "zypper install -y tesseract-ocr tesseract-ocr-traineddata-english"
                .to_string(),
            needs_sudo: true,
            post_install: None,
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },

        LinuxDistro::Unknown => InstallInstructions {
            platform,
            package_manager: None,
            command: "# Please install tesseract-ocr using your distribution's package manager"
                .to_string(),
            needs_sudo: true,
            post_install: Some(
                "After installation, ensure TESSDATA_PREFIX is set if language data is not auto-detected"
                    .to_string(),
            ),
            manual_url: Some(
                "https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string(),
            ),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        },
    }
}

/// Build Windows-specific installation instructions.
fn build_windows_instructions(platform: Platform) -> InstallInstructions {
    // Check for winget first (Windows 10 1709+ / Windows 11)
    if command_exists("winget") {
        InstallInstructions {
            platform,
            package_manager: Some("winget".to_string()),
            command: "winget install -e --id UB-Mannheim.TesseractOCR".to_string(),
            needs_sudo: false,
            post_install: Some(
                "Restart your terminal/PowerShell after installation to update PATH".to_string(),
            ),
            manual_url: Some("https://github.com/UB-Mannheim/tesseract/wiki".to_string()),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        }
    } else if command_exists("choco") {
        InstallInstructions {
            platform,
            package_manager: Some("choco".to_string()),
            command: "choco install tesseract".to_string(),
            needs_sudo: false, // choco should be run in admin PowerShell
            post_install: Some(
                "Run in Administrator PowerShell. Restart terminal after installation.".to_string(),
            ),
            manual_url: Some("https://github.com/UB-Mannheim/tesseract/wiki".to_string()),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        }
    } else {
        InstallInstructions {
            platform,
            package_manager: None,
            command: "# Download installer from https://github.com/UB-Mannheim/tesseract/wiki"
                .to_string(),
            needs_sudo: false,
            post_install: Some(
                "After installation, add Tesseract to your PATH environment variable".to_string(),
            ),
            manual_url: Some("https://github.com/UB-Mannheim/tesseract/wiki".to_string()),
            provides: vec![
                "PDF OCR".to_string(),
                "Scanned document text extraction".to_string(),
            ],
        }
    }
}

/// Build instructions for unknown platforms.
fn build_unknown_instructions(platform: Platform) -> InstallInstructions {
    InstallInstructions {
        platform,
        package_manager: None,
        command: "# Please install tesseract-ocr for your system".to_string(),
        needs_sudo: false,
        post_install: Some(
            "Set TESSDATA_PREFIX environment variable to your tessdata directory".to_string(),
        ),
        manual_url: Some("https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string()),
        provides: vec![
            "PDF OCR".to_string(),
            "Scanned document text extraction".to_string(),
        ],
    }
}
