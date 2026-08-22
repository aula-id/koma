use super::*;

#[test]
fn extract_package_name_test() {
    assert_eq!(extract_package_name("lodash"), "lodash");
    assert_eq!(extract_package_name("lodash/fp"), "lodash");
    assert_eq!(extract_package_name("@scope/pkg"), "@scope/pkg");
    assert_eq!(extract_package_name("@scope/pkg/sub"), "@scope/pkg");
    assert_eq!(extract_package_name("@scope/pkg/sub/deep"), "@scope/pkg");
}

#[test]
fn relative_import_exact() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-jsresolve-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("utils.ts"), "").unwrap();

    let mut known = HashSet::new();
    let utils_path =
        normalize_lexical(&src.join("utils.ts").to_string_lossy().replace('\\', "/"));
    known.insert(utils_path.clone());

    let import_ref = ImportRef {
        specifier: "./utils".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: &normalize_lexical(
            &src.join("main.ts").to_string_lossy().replace('\\', "/"),
        ),
        ts_config: None,
        package_json: None,
        known_files: &known,
        owner_root: &tmp.to_string_lossy().replace('\\', "/"),
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(matches!(res, Resolution::Resolved(ref v) if v[0] == utils_path));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn relative_import_ts_substitution() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-jssub-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("component.tsx"), "").unwrap();

    let mut known = HashSet::new();
    let component_path = normalize_lexical(
        &src.join("component.tsx")
            .to_string_lossy()
            .replace('\\', "/"),
    );
    known.insert(component_path.clone());

    let import_ref = ImportRef {
        specifier: "./component".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: &normalize_lexical(
            &src.join("app.ts").to_string_lossy().replace('\\', "/"),
        ),
        ts_config: None,
        package_json: None,
        known_files: &known,
        owner_root: &tmp.to_string_lossy().replace('\\', "/"),
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(matches!(res, Resolution::Resolved(ref v) if v[0] == component_path));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bare_specifier_is_external() {
    let import_ref = ImportRef {
        specifier: "react".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let known = HashSet::new();
    let ctx = JsTsResolveContext {
        importer_path: "/src/app.ts",
        ts_config: None,
        package_json: None,
        known_files: &known,
        owner_root: "/",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(matches!(res, Resolution::External { ref package } if package == "react"));
}

// ─── Phase 3: owner_root containment tests ────────────────────────

#[test]
fn relative_import_outside_owner_is_outside_workspace() {
    let mut known = HashSet::new();
    // File is outside owner root.
    known.insert("/other/src/util.ts".into());

    let import_ref = ImportRef {
        specifier: "../../other/src/util".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: None,
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(
        matches!(
            res,
            Resolution::Unresolved {
                reason: UnresolvedReason::OutsideWorkspace { .. }
            }
        ),
        "relative import resolving outside owner should be OutsideWorkspace, got: {:?}",
        res
    );
}

#[test]
fn relative_import_within_owner_resolves() {
    let mut known = HashSet::new();
    known.insert("/owner/src/util.ts".into());

    let import_ref = ImportRef {
        specifier: "./util".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: None,
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(
        matches!(res, Resolution::Resolved(ref v) if v[0].ends_with("util.ts")),
        "relative import within owner should resolve, got: {:?}",
        res
    );
}

#[test]
fn baseurl_outside_owner_is_outside_workspace() {
    let mut known = HashSet::new();
    known.insert("/cross-root/lib/helper.ts".into());

    let config = TsConfig {
        base_url_resolved: Some("/cross-root/lib".into()),
        config_dir: "/owner".into(),
        ..Default::default()
    };

    let import_ref = ImportRef {
        specifier: "helper".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: Some(&config),
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(
        matches!(
            res,
            Resolution::Unresolved {
                reason: UnresolvedReason::OutsideWorkspace { .. }
            }
        ),
        "baseUrl outside owner should be OutsideWorkspace, got: {:?}",
        res
    );
}

// ─── Phase 3: moduleResolution mode gate tests ────────────────────

#[test]
fn bundler_mode_allows_extensionless_index() {
    let mut known = HashSet::new();
    // Directory index file exists.
    known.insert("/owner/src/components/index.ts".into());

    let config = TsConfig {
        module_resolution: Some("bundler".into()),
        config_dir: "/owner".into(),
        ..Default::default()
    };

    let import_ref = ImportRef {
        specifier: "./components".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: Some(&config),
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(
        matches!(res, Resolution::Resolved(ref v) if v[0].ends_with("index.ts")),
        "bundler mode should allow directory index resolution, got: {:?}",
        res
    );
}

#[test]
fn node16_mode_requires_explicit_extension_for_index() {
    let mut known = HashSet::new();
    // Only a directory index file exists, no explicit extension.
    known.insert("/owner/src/components/index.ts".into());

    let config = TsConfig {
        module_resolution: Some("node16".into()),
        config_dir: "/owner".into(),
        ..Default::default()
    };

    let import_ref = ImportRef {
        specifier: "./components".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: Some(&config),
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    // node16/nodenext: extensionless + index is only for paths ending in /
    // or explicit index path.  Bare "components" without / should not
    // resolve via index in strict mode.
    assert!(
        matches!(
            res,
            Resolution::Unresolved {
                reason: UnresolvedReason::NotFound
            }
        ),
        "node16 mode should NOT allow bare directory index, got: {:?}",
        res
    );
}

#[test]
fn node16_mode_with_explicit_ts_extension_resolves() {
    let mut known = HashSet::new();
    known.insert("/owner/src/utils.ts".into());

    let config = TsConfig {
        module_resolution: Some("node16".into()),
        config_dir: "/owner".into(),
        ..Default::default()
    };

    let import_ref = ImportRef {
        specifier: "./utils".into(),
        kind: ImportKind::Static,
        span: None,
        condition: None,
    };
    let ctx = JsTsResolveContext {
        importer_path: "/owner/src/app.ts",
        ts_config: Some(&config),
        package_json: None,
        known_files: &known,
        owner_root: "/owner",
    };
    let res = resolve_js_ts_import(&import_ref, &ctx);
    assert!(
        matches!(res, Resolution::Resolved(ref v) if v[0].ends_with("utils.ts")),
        "node16 mode should resolve with TS extension substitution, got: {:?}",
        res
    );
}
