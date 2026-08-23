//! koma-managed language servers (`~/.koma/lsp/`).
//!
//! Host-spawned LSP side of the coding panel: catalogue, PATH/managed resolve,
//! install/uninstall recipes, the `koma lsp` CLI, and the stdio JSON-RPC
//! language client ([`client::LspManager`]) that drives Monaco completion /
//! hover / definition / references / diagnostics.
//!
//! Layout:
//! ```text
//! ~/.koma/lsp/
//!   manifest.json          # id → version/source/installed_at
//!   <id>/
//!     bin/<binary>         # github / go / npm layout
//!     venv/                # pip layout (basedpyright)
//! ```
//!
//! Resolution order: koma-managed → PATH → missing.

pub mod catalog;
pub mod client;
pub mod install;
pub mod manifest;
pub mod resolve;

// Re-exports are the stable surface for GUI host code and LspManager.
// Not every symbol is consumed by the CLI path yet — keep them public.
#[allow(unused_imports)]
pub use catalog::{find, find_by_extension, ServerSpec, CATALOG};
#[allow(unused_imports)]
pub use client::{
    language_id_for_path, LspCompletionItem, LspDiagnostic, LspDocumentSymbol, LspHover,
    LspLocation, LspManager, LspRange, LspRuntimeServer,
};
#[allow(unused_imports)]
pub use install::{install_all, install_one, print_status, uninstall_one, ProgressFn};
#[allow(unused_imports)]
pub use manifest::{lsp_dir, managed_binary_path, server_dir, Manifest, ManifestEntry};
#[allow(unused_imports)]
pub use resolve::{status_all, status_for_extension, status_one, ServerStatus, Source};

/// CLI subcommand parsed from `koma lsp ...`.
#[derive(Debug, Clone)]
pub enum LspCli {
    /// `koma lsp status`
    Status,
    /// `koma lsp install <id>` or `koma lsp install --all`
    Install {
        id: Option<String>,
        all: bool,
        force: bool,
    },
    /// `koma lsp uninstall <id>`
    Uninstall {
        id: String,
    },
    /// Bare / unknown → print usage.
    Usage,
}

impl LspCli {
    /// Parse the tokens AFTER the leading `lsp` verb.
    pub fn parse(args: &[String]) -> Self {
        let mut force = false;
        let mut all = false;
        let mut positionals: Vec<&str> = Vec::new();
        for a in args {
            match a.as_str() {
                "--force" => force = true,
                "--all" => all = true,
                s if s.starts_with("--") => {}
                s => positionals.push(s),
            }
        }
        match positionals.first().copied() {
            Some("status") => LspCli::Status,
            Some("install") => {
                let id = positionals.get(1).map(|s| (*s).to_string());
                if all || id.as_deref() == Some("--all") {
                    LspCli::Install {
                        id: None,
                        all: true,
                        force,
                    }
                } else if let Some(id) = id {
                    LspCli::Install {
                        id: Some(id),
                        all: false,
                        force,
                    }
                } else {
                    LspCli::Usage
                }
            }
            Some("uninstall") => match positionals.get(1) {
                Some(id) => LspCli::Uninstall {
                    id: (*id).to_string(),
                },
                None => LspCli::Usage,
            },
            _ => LspCli::Usage,
        }
    }
}

/// Print `koma lsp` usage to stdout; return exit code 1 (usage error).
pub fn print_usage() -> i32 {
    println!(
        "koma lsp — manage language servers for the coding panel\n\
         \n\
         usage:\n\
         \x20 koma lsp status\n\
         \x20 koma lsp install <id> [--force]\n\
         \x20 koma lsp install --all [--force]\n\
         \x20 koma lsp uninstall <id>\n\
         \n\
         servers land under ~/.koma/lsp/<id>/ (never mutates system packages).\n\
         resolution order: koma-managed → PATH → missing (Monarch-only).\n\
         \n\
         managed ids:\n\
         \x20 rust-analyzer, vtsls, basedpyright, gopls, clangd,\n\
         \x20 vscode-langservers, bash-language-server, taplo\n\
         \x20 (lua-language-server, zls, nil: PATH discovery only for now)"
    );
    1
}

/// Run a parsed [`LspCli`]. Returns the process exit code.
pub fn run_cli(cmd: LspCli) -> i32 {
    match cmd {
        LspCli::Usage => print_usage(),
        LspCli::Status => print_status(),
        LspCli::Install { id, all, force } => {
            let result = if all {
                install_all(force, None)
            } else if let Some(id) = id {
                install_one(&id, force, None)
            } else {
                return print_usage();
            };
            match result {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e:#}");
                    1
                }
            }
        }
        LspCli::Uninstall { id } => match uninstall_one(&id) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {e:#}");
                1
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status() {
        let cmd = LspCli::parse(&["status".into()]);
        assert!(matches!(cmd, LspCli::Status));
    }

    #[test]
    fn parse_install_one() {
        let cmd = LspCli::parse(&["install".into(), "taplo".into()]);
        match cmd {
            LspCli::Install {
                id: Some(id),
                all: false,
                force: false,
            } => assert_eq!(id, "taplo"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_install_all_force() {
        let cmd = LspCli::parse(&["install".into(), "--all".into(), "--force".into()]);
        assert!(matches!(
            cmd,
            LspCli::Install {
                id: None,
                all: true,
                force: true
            }
        ));
    }

    #[test]
    fn parse_uninstall() {
        let cmd = LspCli::parse(&["uninstall".into(), "taplo".into()]);
        match cmd {
            LspCli::Uninstall { id } => assert_eq!(id, "taplo"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_bare_is_usage() {
        assert!(matches!(LspCli::parse(&[]), LspCli::Usage));
        assert!(matches!(
            LspCli::parse(&["nope".into()]),
            LspCli::Usage
        ));
    }
}
