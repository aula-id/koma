//! Go import resolver.
//!
//! Resolves Go imports against go.mod module boundaries, go.work workspace
//! configuration, and vendor directories. Only local package directories
//! and explicit go.mod/go.work replace relationships are valid cross-root paths.

use crate::linker::config::go_mod::GoModuleConfig;
use crate::linker::path::normalize_lexical;
use crate::linker::project::ProjectIndex;
use crate::linker::reference::{ImportMeta, ImportRef, Resolution, UnresolvedReason};
use crate::linker::resolve::ResolveContext;

use std::collections::HashSet;
use std::path::Path;

/// Resolve a Go import reference against the project.
pub fn resolve_go_import(
    import_ref: &ImportRef,
    meta: Option<&ImportMeta>,
    ctx: &ResolveContext<'_>,
) -> Resolution {
    let conditions: &[String] = match meta {
        Some(ImportMeta::Go(m)) => &m.conditions,
        _ => &[],
    };

    // `go:build ignore` → skip resolution.
    if import_ref.condition.as_deref() == Some("go:build ignore") {
        return Resolution::Unresolved {
            reason: UnresolvedReason::UnsupportedSyntax {
                detail: "go:build ignore — file excluded from build".into(),
            },
        };
    }

    // Blank import (`_ "pkg"`): still creates dependency edge.
    // Dot import (`.` "pkg"): creates dependency edge.
    // Both are valid Go import forms.

    let specifier = &import_ref.specifier;

    // Relative import (./ or ../).
    if specifier.starts_with("./") || specifier.starts_with("../") {
        return resolve_relative_import(specifier, ctx, conditions);
    }

    // Get Go module config for the importer's owner.
    let go_config = ctx.project.go_module_config_for_importer(ctx.importer);

    // Standard library check: single-segment path or known stdlib.
    if is_stdlib_import(specifier) {
        // Check if vendored locally.
        if let Some(config) = go_config {
            if config.vendor_mode {
                if let Some(vendor_path) = resolve_in_vendor(specifier, ctx.importer, ctx.project) {
                    return Resolution::Resolved(vendor_path);
                }
            }
        }
        return Resolution::External {
            package: specifier.to_string(),
        };
    }

    // Check go.work replace directives.
    if let Some(config) = go_config {
        if let Some(work) = &config.work {
            if let Some(rep) = work.replaces.get(specifier) {
                if rep.local {
                    // Local replace: resolve within the replace target.
                    return resolve_local_replace(&rep.new, specifier, ctx, conditions);
                }
            }
        }
    }

    // Check go.mod replace directives for the nearest module.
    if let Some(config) = go_config {
        if let Some(mod_config) = find_nearest_module(ctx.importer, config) {
            if let Some(rep) = mod_config.replaces.get(specifier) {
                if rep.local {
                    return resolve_local_replace(&rep.new, specifier, ctx, conditions);
                }
            }
        }
    }

    // Check if the import is within the same module.
    if let Some(config) = go_config {
        if let Some(mod_config) = find_nearest_module(ctx.importer, config) {
            if !mod_config.module_path.is_empty() && specifier.starts_with(&mod_config.module_path)
            {
                // Same module: resolve to local package.
                let suffix = &specifier[mod_config.module_path.len()..];
                if suffix.is_empty() || suffix.starts_with('/') {
                    // Find the module root directory.
                    if let Some(mod_dir) = find_module_dir(ctx.importer, config) {
                        let local_path = if suffix.is_empty() {
                            mod_dir
                        } else {
                            let sub = &suffix[1..]; // strip leading /
                            normalize_lexical(&format!("{mod_dir}/{sub}"))
                        };
                        let known = ctx.project.known_file_set();
                        let targets = resolve_go_package(&local_path, known);
                        if !targets.is_empty() {
                            return Resolution::Resolved(targets);
                        }
                    }
                }
            }
        }
    }

    // External module: check vendor if in vendor mode.
    if let Some(config) = go_config {
        if config.vendor_mode {
            if let Some(vendor_path) = resolve_in_vendor(specifier, ctx.importer, ctx.project) {
                return Resolution::Resolved(vendor_path);
            }
        }
    }

    // Truly external module.
    Resolution::External {
        package: specifier.to_string(),
    }
}

/// Resolve a relative Go import (./ or ../).
fn resolve_relative_import(
    specifier: &str,
    ctx: &ResolveContext<'_>,
    conditions: &[String],
) -> Resolution {
    let importer_dir = Path::new(ctx.importer)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "/".into());

    let resolved = normalize_lexical(&format!("{importer_dir}/{specifier}"));
    let owner = normalize_lexical(ctx.project.file_owner(ctx.importer).unwrap_or("/"));

    if !resolved.starts_with(&owner) {
        return Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace {
                normalized_path: resolved,
            },
        };
    }

    // Try as directory with Go files.
    let known = ctx.project.known_file_set();
    let targets = resolve_go_package(&resolved, known);
    if !targets.is_empty() {
        let _ = conditions; // conditions preserved via meta at scan level
        return Resolution::Resolved(targets);
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve a local replace directive target.
fn resolve_local_replace(
    new_path: &str,
    specifier: &str,
    ctx: &ResolveContext<'_>,
    conditions: &[String],
) -> Resolution {
    let _ = conditions;
    // Find the module root containing the importer.
    let go_config = ctx.project.go_module_config_for_importer(ctx.importer);
    let mod_dir = go_config.and_then(|config| find_module_dir(ctx.importer, config));

    let base = mod_dir
        .unwrap_or_else(|| normalize_lexical(ctx.project.file_owner(ctx.importer).unwrap_or("/")));

    // The replace target is relative to the module root.
    let local_base = normalize_lexical(&format!("{base}/{new_path}"));

    // Compute the import suffix after the old module path.
    let suffix = if let Some(config) = go_config {
        if let Some(mod_config) = find_nearest_module(ctx.importer, config) {
            if specifier.starts_with(&mod_config.module_path) {
                &specifier[mod_config.module_path.len()..]
            } else {
                ""
            }
        } else {
            ""
        }
    } else {
        ""
    };

    let target_dir = if suffix.is_empty() || suffix == "/" {
        local_base
    } else {
        normalize_lexical(&format!("{local_base}/{}", &suffix[1..]))
    };

    let known = ctx.project.known_file_set();
    let targets = resolve_go_package(&target_dir, known);
    if !targets.is_empty() {
        return Resolution::Resolved(targets);
    }

    Resolution::Unresolved {
        reason: UnresolvedReason::NotFound,
    }
}

/// Resolve a Go package directory to all non-test .go source files.
///
/// Creates edges to every non-test .go source file in the package under
/// conservative all-platform context. Excludes *_test.go files.
fn resolve_go_package(package_dir: &str, known_files: &HashSet<String>) -> Vec<String> {
    let mut targets = Vec::new();

    // Check if the directory exists and contains .go files.
    let dir = Path::new(package_dir);
    if !dir.is_dir() {
        return targets;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            if !name_s.ends_with(".go") {
                continue;
            }
            // Exclude test files.
            if name_s.ends_with("_test.go") {
                continue;
            }
            let path = normalize_lexical(&entry.path().to_string_lossy().replace('\\', "/"));
            if known_files.contains(&path) {
                targets.push(path);
            }
        }
    }

    // Also check for go files in known_files if the directory walk missed them.
    let prefix = format!("{package_dir}/");
    for file in known_files {
        if file.starts_with(&prefix)
            && file.ends_with(".go")
            && !file.ends_with("_test.go")
            && !targets.contains(file)
        {
            // Verify the file is directly in the package dir (not subdirectory).
            let rel = &file[prefix.len()..];
            if !rel.contains('/') {
                targets.push(file.clone());
            }
        }
    }

    targets
}

/// Resolve a Go import from a vendor directory.
///
/// Only valid when go metadata indicates vendor mode.
fn resolve_in_vendor(
    specifier: &str,
    importer: &str,
    project: &ProjectIndex,
) -> Option<Vec<String>> {
    // Find the module root for the importer.
    let go_config = project.go_module_config_for_importer(importer)?;
    let mod_dir = find_module_dir(importer, go_config)?;

    let vendor_dir = format!("{mod_dir}/vendor/{specifier}");
    let known = project.known_file_set();
    let targets = resolve_go_package(&vendor_dir, known);
    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}

/// Find the nearest go.mod module directory for an importer file.
fn find_nearest_module<'a>(
    importer: &str,
    config: &'a GoModuleConfig,
) -> Option<&'a crate::linker::config::go_mod::GoModConfig> {
    let importer_path = Path::new(importer);
    let mut best: Option<(&str, &crate::linker::config::go_mod::GoModConfig)> = None;

    for (mod_dir, mod_config) in &config.mods {
        if importer_path.starts_with(mod_dir.as_str())
            && best.is_none_or(|b: (&str, _)| mod_dir.len() > b.0.len())
        {
            best = Some((mod_dir.as_str(), mod_config));
        }
    }

    best.map(|(_, cfg)| cfg)
}

/// Find the directory containing the nearest go.mod for an importer.
fn find_module_dir(importer: &str, config: &GoModuleConfig) -> Option<String> {
    let importer_path = Path::new(importer);
    let mut best: Option<&str> = None;

    for mod_dir in config.mods.keys() {
        if importer_path.starts_with(mod_dir.as_str())
            && best.is_none_or(|b| mod_dir.len() > b.len())
        {
            best = Some(mod_dir.as_str());
        }
    }

    best.map(String::from)
}

/// Determine if an import path is a Go standard library package.
///
/// Heuristic: a standard library import has no dots in any path segment
/// (except the first), no `.` domain, and is a well-known single-segment
/// or well-known multi-segment path.
fn is_stdlib_import(specifier: &str) -> bool {
    // Relative imports are not stdlib.
    if specifier.starts_with('.') {
        return false;
    }

    let segments: Vec<&str> = specifier.split('/').collect();
    if segments.is_empty() {
        return false;
    }

    // Well-known stdlib packages.
    if matches!(
        segments[0],
        "fmt"
            | "os"
            | "io"
            | "net"
            | "math"
            | "sync"
            | "time"
            | "sort"
            | "bytes"
            | "strings"
            | "errors"
            | "context"
            | "testing"
            | "runtime"
            | "log"
            | "flag"
            | "path"
            | "filepath"
            | "strconv"
            | "unicode"
            | "encoding"
            | "compress"
            | "database"
            | "debug"
            | "go"
            | "hash"
            | "index"
            | "internal"
            | "mime"
            | "plugin"
            | "reflect"
            | "regexp"
            | "syscall"
            | "unsafe"
    ) {
        return true;
    }

    // Standard library: no dots in the first segment (domain-like).
    if segments[0].contains('.') {
        return false;
    }

    // Known stdlib sub-packages.
    if matches!(
        specifier,
        "os/exec"
            | "os/signal"
            | "os/user"
            | "io/ioutil"
            | "net/http"
            | "net/url"
            | "net/http/httptest"
            | "encoding/json"
            | "encoding/xml"
            | "encoding/csv"
            | "encoding/binary"
            | "path/filepath"
            | "sync/atomic"
            | "crypto/sha256"
            | "crypto/md5"
            | "crypto/tls"
            | "crypto/x509"
            | "text/template"
            | "text/scanner"
            | "container/list"
            | "container/heap"
            | "container/ring"
            | "go/ast"
            | "go/parser"
            | "go/token"
            | "go/format"
            | "go/scanner"
            | "go/types"
            | "go/importer"
            | "go/doc"
            | "go/printer"
            | "go/build"
            | "go/constant"
            | "html/template"
            | "html"
            | "log/syslog"
            | "log/slog"
            | "math/big"
            | "math/bits"
            | "math/cmplx"
            | "math/rand"
            | "runtime/cgo"
            | "runtime/debug"
            | "runtime/pprof"
            | "runtime/trace"
            | "runtime/metrics"
            | "unicode/utf8"
            | "unicode/utf16"
            | "regexp/syntax"
            | "database/sql"
            | "database/sql/driver"
            | "database/sql/dsn"
            | "debug/dwarf"
            | "debug/elf"
            | "debug/gosym"
            | "debug/macho"
            | "debug/pe"
            | "debug/plan9obj"
            | "compress/gzip"
            | "compress/zlib"
            | "compress/bzip2"
            | "compress/flate"
            | "compress/lzw"
            | "compress/snappy"
            | "compress/zstd"
            | "hash/crc32"
            | "hash/crc64"
            | "hash/fnv"
            | "hash/maphash"
            | "index/suffixarray"
            | "internal/coverage"
            | "internal/poll"
            | "internal/safeio"
            | "internal/syscall/unix"
            | "internal/testenv"
            | "internal/trace"
            | "mime/multipart"
            | "mime/quotedprintable"
            | "plugin"
            | "sort/slice"
            | "strconv/quote"
            | "strings/strings"
            | "strings/bytes"
            | "strings/reader"
            | "sync/maps"
            | "testing/fstest"
            | "testing/iotest"
            | "testing/quick"
            | "testing/slogtest"
            | "unicode/binary"
            | "unicode/utf8/utf8string"
    ) {
        return true;
    }

    // Single-segment with no dots: could be stdlib or internal package.
    // Conservative: treat single-segment non-dot imports as potential stdlib.
    segments.len() == 1 && !segments[0].contains('.')
}

#[cfg(test)]
#[path = "go_test.rs"]
mod tests;
