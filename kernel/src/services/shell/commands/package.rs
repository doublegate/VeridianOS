//! Package management commands.

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String};

use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

pub(in crate::services::shell) struct PkgCommand;
impl BuiltinCommand for PkgCommand {
    fn name(&self) -> &str {
        "pkg"
    }
    fn description(&self) -> &str {
        "Package management (install, remove, update, upgrade, list, search, info, verify)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(
                "Usage: pkg <install|remove|update|upgrade|list|search|info|verify> [args...]",
            ));
        }

        let subcommand = args[0].as_str();
        let sub_args = &args[1..];

        match subcommand {
            "install" => pkg_install(sub_args),
            "remove" => pkg_remove(sub_args),
            "update" => pkg_update(),
            "upgrade" => pkg_upgrade(sub_args),
            "list" => pkg_list(sub_args),
            "search" => pkg_search(sub_args),
            "info" => pkg_info(sub_args),
            "verify" => pkg_verify(sub_args),
            _ => CommandResult::Error(format!("pkg: unknown subcommand '{}'", subcommand)),
        }
    }
}

/// Install a package by name
fn pkg_install(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg install <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.install(name.clone(), String::from("*"))) {
        Some(Ok(())) => {
            crate::println!("Package '{}' installed successfully", name);
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg install: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Remove an installed package
fn pkg_remove(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg remove <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.remove(name)) {
        Some(Ok(())) => {
            crate::println!("Package '{}' removed successfully", name);
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg remove: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Refresh repository index
fn pkg_update() -> CommandResult {
    match crate::pkg::with_package_manager(|mgr| mgr.update()) {
        Some(Ok(())) => {
            crate::println!("Package index updated");
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg update: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Upgrade packages (reinstall with latest version)
fn pkg_upgrade(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from(
            "Usage: pkg upgrade <name> or pkg upgrade --all",
        ));
    }

    let target = args[0].as_str();

    if target == "--all" {
        // Upgrade all: remove and reinstall each package
        match crate::pkg::with_package_manager(|mgr| {
            let packages = mgr.list_installed();
            let count = packages.len();
            for (name, _version) in &packages {
                crate::println!("  Upgrading {} ...", name);
                // Remove and reinstall to get the latest version
                let _ = mgr.remove(name);
                let _ = mgr.install(name.clone(), String::from("*"));
            }
            count
        }) {
            Some(count) => {
                crate::println!("Upgraded {} package(s)", count);
                CommandResult::Success(0)
            }
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    } else {
        match crate::pkg::with_package_manager(|mgr| {
            mgr.remove(&String::from(target))?;
            mgr.install(String::from(target), String::from("*"))
        }) {
            Some(Ok(())) => {
                crate::println!("Package '{}' upgraded successfully", target);
                CommandResult::Success(0)
            }
            Some(Err(e)) => CommandResult::Error(format!("pkg upgrade: {}", e)),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    }
}

/// List packages
fn pkg_list(args: &[String]) -> CommandResult {
    let filter = args.first().map(|s| s.as_str()).unwrap_or("--installed");

    match filter {
        "--installed" => match crate::pkg::with_package_manager(|mgr| mgr.list_installed()) {
            Some(packages) => {
                if packages.is_empty() {
                    crate::println!("No packages installed");
                } else {
                    crate::println!("Installed packages:");
                    for (name, version) in &packages {
                        crate::println!(
                            "  {} {}.{}.{}",
                            name,
                            version.major,
                            version.minor,
                            version.patch
                        );
                    }
                    crate::println!("{} package(s) installed", packages.len());
                }
                CommandResult::Success(0)
            }
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        },
        "--available" => {
            crate::println!("Available packages (from repositories):");
            crate::println!("  Run 'pkg update' first to refresh the index");
            CommandResult::Success(0)
        }
        _ => CommandResult::Error(format!(
            "pkg list: unknown filter '{}' (use --installed or --available)",
            filter
        )),
    }
}

/// Search installed packages by name substring
fn pkg_search(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg search <query>"));
    }

    let query = &args[0];
    match crate::pkg::with_package_manager(|mgr| {
        let packages = mgr.list_installed();
        let mut count = 0usize;
        for (name, version) in &packages {
            if name.contains(query.as_str()) {
                crate::println!(
                    "  {} {}.{}.{}",
                    name,
                    version.major,
                    version.minor,
                    version.patch
                );
                count += 1;
            }
        }
        count
    }) {
        Some(0) => {
            crate::println!("No packages found matching '{}'", query);
            CommandResult::Success(0)
        }
        Some(count) => {
            crate::println!("{} result(s)", count);
            CommandResult::Success(0)
        }
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Show package details
fn pkg_info(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg info <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.get_metadata(name).cloned()) {
        Some(Some(meta)) => {
            crate::println!("Package: {}", meta.name);
            crate::println!(
                "Version: {}.{}.{}",
                meta.version.major,
                meta.version.minor,
                meta.version.patch
            );
            crate::println!("Author:  {}", meta.author);
            crate::println!("License: {}", meta.license);
            crate::println!("Description: {}", meta.description);
            crate::println!("Installed: yes");
            if !meta.dependencies.is_empty() {
                crate::println!("Dependencies:");
                for dep in &meta.dependencies {
                    crate::println!("  {} ({})", dep.name, dep.version_req);
                }
            }
            CommandResult::Success(0)
        }
        Some(None) => CommandResult::Error(format!("pkg info: package '{}' not found", name)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Verify package is installed
fn pkg_verify(args: &[String]) -> CommandResult {
    if args.is_empty() {
        // Verify all installed packages
        match crate::pkg::with_package_manager(|mgr| {
            let packages = mgr.list_installed();
            for (name, version) in &packages {
                crate::println!(
                    "  {} {}.{}.{} ... OK",
                    name,
                    version.major,
                    version.minor,
                    version.patch
                );
            }
            crate::println!("Verified: {} package(s)", packages.len());
            packages.len()
        }) {
            Some(_count) => CommandResult::Success(0),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    } else {
        let name = &args[0];
        match crate::pkg::with_package_manager(|mgr| mgr.is_installed(name)) {
            Some(true) => {
                crate::println!("Package '{}': OK (installed)", name);
                CommandResult::Success(0)
            }
            Some(false) => CommandResult::Error(format!("pkg verify: '{}' is not installed", name)),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    }
}
