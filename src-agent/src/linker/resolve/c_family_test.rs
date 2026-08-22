use super::*;
use crate::linker::reference::ByteSpan;

fn import(specifier: &str, kind: ImportKind) -> ImportRef {
    ImportRef {
        specifier: specifier.into(),
        kind,
        span: Some(ByteSpan { start: 0, end: 1 }),
        condition: None,
    }
}

fn context<'a>(
    flags: Option<&'a CompileFlags>,
    files: &'a HashSet<String>,
) -> CFamilyResolveContext<'a> {
    CFamilyResolveContext {
        importer_path: "/owner/src/main.c",
        compile_flags: flags,
        known_files: files,
        owner_root: "/owner",
    }
}

#[test]
fn search_order_matches_compiler_rules() {
    let flags = CompileFlags {
        language_mode: None,
        iquote: vec!["/q".into()],
        include_paths: vec!["/i".into()],
        isystem: vec!["/s".into()],
    };
    assert_eq!(
        build_search_paths(true, "/src", Some(&flags)),
        vec!["/src", "/q", "/i"]
    );
    assert_eq!(
        build_search_paths(false, "/src", Some(&flags)),
        vec!["/i", "/s"]
    );
}

#[test]
fn no_guessed_versioned_system_paths() {
    assert!(build_search_paths(false, "/src", None).is_empty());
}

#[test]
fn escaping_owner_is_outside_even_when_other_root_knows_file() {
    let files = HashSet::from(["/other/secret.h".to_string()]);
    let result = resolve_c_include(
        &import("../../other/secret.h", ImportKind::IncludeQuoted),
        &context(None, &files),
    );
    assert!(matches!(
        result,
        Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace { .. }
        }
    ));
}

#[test]
fn missing_headers_are_classified_precisely() {
    let files = HashSet::new();
    assert!(matches!(
        resolve_c_include(
            &import("vector", ImportKind::IncludeAngle),
            &context(None, &files)
        ),
        Resolution::External { .. }
    ));
    assert!(matches!(
        resolve_c_include(
            &import("sdk/header.h", ImportKind::IncludeAngle),
            &context(None, &files)
        ),
        Resolution::Unresolved {
            reason: UnresolvedReason::ConfigRequired { .. }
        }
    ));
    assert!(matches!(
        resolve_c_include(
            &import("missing.h", ImportKind::IncludeQuoted),
            &context(None, &files)
        ),
        Resolution::Unresolved {
            reason: UnresolvedReason::NotFound
        }
    ));
}

#[test]
fn configured_missing_local_angle_is_not_found() {
    let files = HashSet::new();
    let flags = CompileFlags {
        include_paths: vec!["/owner/include".into()],
        ..CompileFlags::default()
    };
    assert!(matches!(
        resolve_c_include(
            &import("local/header.h", ImportKind::IncludeAngle),
            &context(Some(&flags), &files)
        ),
        Resolution::Unresolved {
            reason: UnresolvedReason::NotFound
        }
    ));
}

#[test]
fn named_module_requires_mapping() {
    let files = HashSet::new();
    assert!(matches!(
        resolve_c_include(
            &import("std.core", ImportKind::ModuleDecl),
            &context(None, &files)
        ),
        Resolution::Unresolved {
            reason: UnresolvedReason::ConfigRequired { .. }
        }
    ));
}

#[test]
fn detects_hh_and_ambiguous_h() {
    assert_eq!(
        detect_header_language("foo.hh", None),
        Some(crate::linker::graph::Lang::Cpp)
    );
    assert_eq!(detect_header_language("foo.h", None), None);
}
