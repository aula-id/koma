use super::*;

#[test]
fn import_kind_display() {
    assert_eq!(ImportKind::Static.to_string(), "Static");
    assert_eq!(ImportKind::TypeOnly.to_string(), "TypeOnly");
    assert_eq!(ImportKind::ReExport.to_string(), "ReExport");
    assert_eq!(ImportKind::SideEffect.to_string(), "SideEffect");
    assert_eq!(ImportKind::Dynamic.to_string(), "Dynamic");
    assert_eq!(ImportKind::ModuleDecl.to_string(), "ModuleDecl");
    assert_eq!(ImportKind::IncludeQuoted.to_string(), "IncludeQuoted");
    assert_eq!(ImportKind::IncludeAngle.to_string(), "IncludeAngle");
    assert_eq!(ImportKind::PackageImport.to_string(), "PackageImport");
    assert_eq!(ImportKind::Part.to_string(), "Part");
    assert_eq!(ImportKind::PartOf.to_string(), "PartOf");
    assert_eq!(ImportKind::ModuleRequires.to_string(), "ModuleRequires");
}

#[test]
fn import_ref_serde_roundtrip() {
    let r = ImportRef {
        specifier: "crate::foo".into(),
        kind: ImportKind::Static,
        span: Some(ByteSpan { start: 0, end: 14 }),
        condition: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ImportRef = serde_json::from_str(&json).unwrap();
    assert_eq!(r, r2);
}

#[test]
fn import_ref_serde_backward_compat_no_span() {
    // Old JSON without span/condition fields should deserialize via serde(default).
    let json = r#"{"specifier":"crate::foo","kind":"Static"}"#;
    let r: ImportRef = serde_json::from_str(json).unwrap();
    assert_eq!(r.specifier, "crate::foo");
    assert_eq!(r.kind, ImportKind::Static);
    assert!(r.span.is_none());
    assert!(r.condition.is_none());
}

#[test]
fn resolution_serde_roundtrip() {
    let resolutions = vec![
        Resolution::Resolved(vec!["/a.rs".into(), "/b.rs".into()]),
        Resolution::External {
            package: "serde".into(),
        },
        Resolution::Ambiguous {
            candidates: vec!["/x.rs".into(), "/y.rs".into()],
        },
        Resolution::Dynamic {
            expression: "import(foo)".into(),
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::NotFound,
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::OutsideWorkspace {
                normalized_path: "/tmp/evil.rs".into(),
            },
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::MultipleCandidates {
                paths: vec!["/a.rs".into(), "/b.rs".into()],
            },
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::ParseError {
                detail: "bad syntax".into(),
            },
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::UnsupportedSyntax {
                detail: "not yet".into(),
            },
        },
    ];
    for res in &resolutions {
        let json = serde_json::to_string(res).unwrap();
        let back: Resolution = serde_json::from_str(&json).unwrap();
        assert_eq!(res, &back);
    }
}

#[test]
fn unresolved_reason_display() {
    assert_eq!(UnresolvedReason::NotFound.to_string(), "not found");
    assert_eq!(
        UnresolvedReason::OutsideWorkspace {
            normalized_path: "/tmp/evil.rs".into()
        }
        .to_string(),
        "escapes workspace: /tmp/evil.rs"
    );
    assert_eq!(
        UnresolvedReason::MultipleCandidates {
            paths: vec!["/a.rs".into(), "/b.rs".into()]
        }
        .to_string(),
        "ambiguous: 2 candidates"
    );
    assert_eq!(
        UnresolvedReason::ParseError {
            detail: "bad".into()
        }
        .to_string(),
        "parse error: bad"
    );
    assert_eq!(
        UnresolvedReason::UnsupportedSyntax {
            detail: "nope".into()
        }
        .to_string(),
        "unsupported: nope"
    );
}

#[test]
fn source_refs_counts_and_serde_roundtrip() {
    let mut refs = SourceRefs::default();
    refs.push(
        ImportRef {
            specifier: "a".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::Resolved(vec!["/a.rs".into()]),
    );
    refs.push(
        ImportRef {
            specifier: "b".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::External {
            package: "b".into(),
        },
    );
    refs.push(
        ImportRef {
            specifier: "c".into(),
            kind: ImportKind::Dynamic,
            span: None,
            condition: None,
        },
        Resolution::Dynamic {
            expression: "c".into(),
        },
    );
    refs.push(
        ImportRef {
            specifier: "d".into(),
            kind: ImportKind::Static,
            span: None,
            condition: None,
        },
        Resolution::Unresolved {
            reason: UnresolvedReason::NotFound,
        },
    );

    assert_eq!(refs.resolved_count(), 1);
    assert_eq!(refs.external_count(), 1);
    assert_eq!(refs.dynamic_count(), 1);
    assert_eq!(refs.unresolved_count(), 1);
    assert_eq!(refs.ambiguous_count(), 0);
    let json = serde_json::to_string(&refs).unwrap();
    assert_eq!(serde_json::from_str::<SourceRefs>(&json).unwrap(), refs);
}

#[test]
fn unresolved_reason_serde_backward_compat() {
    // Unit variant serializes as a plain string in externally-tagged format.
    let json = r#""NotFound""#;
    let reason: UnresolvedReason = serde_json::from_str(json).unwrap();
    assert_eq!(reason, UnresolvedReason::NotFound);

    // Struct variant with minimal fields still works.
    let json = r#"{"ParseError":{"detail":"bad"}}"#;
    let reason: UnresolvedReason = serde_json::from_str(json).unwrap();
    assert_eq!(
        reason,
        UnresolvedReason::ParseError {
            detail: "bad".into()
        }
    );
}

#[test]
fn unresolved_reason_config_required_serde() {
    let reason = UnresolvedReason::ConfigRequired {
        detail: "compile_commands.json needed".into(),
    };
    let json = serde_json::to_string(&reason).unwrap();
    let back: UnresolvedReason = serde_json::from_str(&json).unwrap();
    assert_eq!(reason, back);
}

#[test]
fn unresolved_reason_package_not_exported_serde() {
    let reason = UnresolvedReason::PackageNotExported {
        package: "lodash".into(),
        subpath: Some("deep/get".into()),
    };
    let json = serde_json::to_string(&reason).unwrap();
    let back: UnresolvedReason = serde_json::from_str(&json).unwrap();
    assert_eq!(reason, back);

    let reason2 = UnresolvedReason::PackageNotExported {
        package: "foo".into(),
        subpath: None,
    };
    let json2 = serde_json::to_string(&reason2).unwrap();
    let back2: UnresolvedReason = serde_json::from_str(&json2).unwrap();
    assert_eq!(reason2, back2);
}

#[test]
fn unresolved_reason_unsupported_config_serde() {
    let reason = UnresolvedReason::UnsupportedConfig {
        path: "tsconfig.json".into(),
        detail: "JSONC not supported".into(),
    };
    let json = serde_json::to_string(&reason).unwrap();
    let back: UnresolvedReason = serde_json::from_str(&json).unwrap();
    assert_eq!(reason, back);
}

#[test]
fn source_refs_serde_roundtrip() {
    let mut sr = SourceRefs::default();
    sr.push(
        ImportRef {
            specifier: "./foo".into(),
            kind: ImportKind::SideEffect,
            span: None,
            condition: Some("cfg(test)".into()),
        },
        Resolution::Resolved(vec!["/project/src/foo.rs".into()]),
    );
    let json = serde_json::to_string(&sr).unwrap();
    let back: SourceRefs = serde_json::from_str(&json).unwrap();
    assert_eq!(sr, back);
}
