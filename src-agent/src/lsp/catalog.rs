//! First-wave language-server catalogue.
//!
//! This is the static map koma ships: id → human name, binary name, extensions,
//! and install recipe kind. The GUI Settings "Language servers" section and the
//! `koma lsp` CLI both render from [`CATALOG`]. Install recipes live in
//! [`super::install`]; this module is pure data + lookup helpers.

/// How a server binary is provisioned under `~/.koma/lsp/<id>/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// GitHub release gzip single binary (rust-analyzer, taplo).
    GithubGz,
    /// GitHub release zip containing a `bin/<name>` (clangd).
    GithubZip,
    /// `npm i -g --prefix ~/.koma/lsp/<id> <pkg>` (needs Node/npm).
    Npm,
    /// Isolated venv + `pip install <pkg>` (needs Python 3).
    PipVenv,
    /// `GOBIN=... go install <module>@latest` (needs Go).
    GoInstall,
}

/// One first-wave language server entry.
#[derive(Debug, Clone, Copy)]
pub struct ServerSpec {
    /// Stable id used on disk (`~/.koma/lsp/<id>/`) and in CLI/IPC.
    pub id: &'static str,
    /// Human-readable label for Settings / CLI status.
    pub name: &'static str,
    /// Executable basename expected on PATH or under the managed install.
    pub binary: &'static str,
    /// File extensions this server owns (no leading dot). Empty = no lazy banner.
    pub extensions: &'static [&'static str],
    /// How the managed installer provisions this server.
    pub kind: InstallKind,
    /// Package / module / release-repo hint shown in status and used by installers.
    ///
    /// - Github*: `"owner/repo"`
    /// - Npm: npm package name
    /// - PipVenv: pip package name
    /// - GoInstall: `module@latest` path (without the `@latest` suffix stored here)
    pub package: &'static str,
    /// Extra argv after the binary when spawning (e.g. `["--stdio"]` for node CLIs).
    /// Reserved for the future host-spawned LspManager; unused by install/resolve.
    #[allow(dead_code)]
    pub args: &'static [&'static str],
}

/// First-wave catalogue (~13 binaries covering ~20 filetypes).
///
/// Order is the Settings / CLI display order.
pub const CATALOG: &[ServerSpec] = &[
    ServerSpec {
        id: "rust-analyzer",
        name: "Rust Analyzer",
        binary: "rust-analyzer",
        extensions: &["rs"],
        kind: InstallKind::GithubGz,
        package: "rust-lang/rust-analyzer",
        args: &[],
    },
    ServerSpec {
        id: "vtsls",
        name: "VTSLS (TypeScript / JavaScript)",
        binary: "vtsls",
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        kind: InstallKind::Npm,
        package: "@vtsls/language-server",
        args: &["--stdio"],
    },
    ServerSpec {
        id: "basedpyright",
        name: "BasedPyright",
        binary: "basedpyright-langserver",
        extensions: &["py", "pyi"],
        kind: InstallKind::PipVenv,
        package: "basedpyright",
        args: &["--stdio"],
    },
    ServerSpec {
        id: "gopls",
        name: "gopls",
        binary: "gopls",
        extensions: &["go"],
        kind: InstallKind::GoInstall,
        package: "golang.org/x/tools/gopls",
        args: &[],
    },
    ServerSpec {
        id: "clangd",
        name: "clangd",
        binary: "clangd",
        extensions: &["c", "h", "hpp", "hh", "cpp", "cc", "cxx"],
        kind: InstallKind::GithubZip,
        package: "clangd/clangd",
        args: &[],
    },
    ServerSpec {
        id: "vscode-langservers",
        name: "vscode-langservers (JSON / HTML / CSS)",
        // Multi-binary package; binary field is the primary (json). Installer
        // also exposes vscode-html-language-server and vscode-css-language-server.
        binary: "vscode-json-language-server",
        extensions: &["json", "jsonc", "html", "htm", "xhtml", "css", "scss", "less"],
        kind: InstallKind::Npm,
        package: "vscode-langservers-extracted",
        args: &["--stdio"],
    },
    ServerSpec {
        id: "bash-language-server",
        name: "Bash Language Server",
        binary: "bash-language-server",
        extensions: &["sh", "bash", "zsh"],
        kind: InstallKind::Npm,
        package: "bash-language-server",
        args: &["start"],
    },
    ServerSpec {
        id: "taplo",
        name: "Taplo (TOML)",
        binary: "taplo",
        extensions: &["toml"],
        kind: InstallKind::GithubGz,
        package: "tamasfe/taplo",
        args: &["lsp", "stdio"],
    },
    ServerSpec {
        id: "lua-language-server",
        name: "Lua Language Server",
        binary: "lua-language-server",
        extensions: &["lua"],
        kind: InstallKind::GithubZip,
        package: "LuaLS/lua-language-server",
        args: &[],
    },
    ServerSpec {
        id: "zls",
        name: "ZLS (Zig)",
        binary: "zls",
        extensions: &["zig", "zon"],
        kind: InstallKind::GithubGz,
        package: "zigtools/zls",
        args: &[],
    },
    ServerSpec {
        id: "nil",
        name: "nil (Nix)",
        binary: "nil",
        extensions: &["nix"],
        kind: InstallKind::GithubGz,
        package: "oxalica/nil",
        args: &[],
    },
];

/// Look up a catalogue entry by id.
pub fn find(id: &str) -> Option<&'static ServerSpec> {
    CATALOG.iter().find(|s| s.id == id)
}

/// Look up the catalogue entry that owns a file extension (no leading dot).
pub fn find_by_extension(ext: &str) -> Option<&'static ServerSpec> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    CATALOG
        .iter()
        .find(|s| s.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
}

/// All known catalogue ids (for `--all` install / status).
#[allow(dead_code)]
pub fn ids() -> impl Iterator<Item = &'static str> {
    CATALOG.iter().map(|s| s.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for s in CATALOG {
            assert!(seen.insert(s.id), "duplicate id {}", s.id);
        }
    }

    #[test]
    fn find_by_extension_rust() {
        let s = find_by_extension("rs").expect("rs");
        assert_eq!(s.id, "rust-analyzer");
    }

    #[test]
    fn find_by_extension_typescript() {
        let s = find_by_extension("tsx").expect("tsx");
        assert_eq!(s.id, "vtsls");
    }

    #[test]
    fn find_unknown_is_none() {
        assert!(find("nope").is_none());
        assert!(find_by_extension("xyzzy").is_none());
    }
}
