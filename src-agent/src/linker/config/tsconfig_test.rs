use super::*;

#[test]
fn strip_jsonc_comments() {
    let input = r#"{
        // This is a comment
        "baseUrl": ".",
        /* block
           comment */
        "paths": {
            "@/*": ["src/*"],
        }
    }"#;
    let result = strip_jsonc(input);
    assert!(!result.contains("comment"));
    assert!(result.contains("\"baseUrl\""));
    assert!(result.contains("\"@/*\""));
}

#[test]
fn parse_jsonc_valid() {
    let input = r#"{
        // comment
        "compilerOptions": {
            "baseUrl": "./src",
            "paths": {
                "@app/*": ["app/*"]
            },
            "moduleResolution": "bundler",
        },
    }"#;
    let value = parse_jsonc(input).unwrap();
    let opts = value.get("compilerOptions").unwrap();
    assert_eq!(opts.get("baseUrl").unwrap().as_str(), Some("./src"));
    assert_eq!(
        opts.get("moduleResolution").unwrap().as_str(),
        Some("bundler")
    );
}

#[test]
fn parse_tsconfig_file_test() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-tsconfig-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("tsconfig.json"),
        r#"{
            "compilerOptions": {
                "baseUrl": "./src",
                "paths": { "@/*": ["*"] },
                "moduleResolution": "node16"
            }
        }"#,
    )
    .unwrap();
    let config = parse_tsconfig_file(&tmp.join("tsconfig.json"), &tmp).unwrap();
    assert_eq!(config.base_url.as_deref(), Some("./src"));
    assert_eq!(config.paths.len(), 1);
    assert_eq!(config.paths[0].0, "@/*");
    assert_eq!(config.module_resolution.as_deref(), Some("node16"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_paths_test() {
    let config = TsConfig {
        paths: vec![
            ("@/*".into(), vec!["src/*".into()]),
            ("@lib/*".into(), vec!["lib/*".into(), "shared/*".into()]),
        ],
        ..Default::default()
    };
    // "@/*" pattern: prefix = "@/", suffix = "". Matches "@/<rest>".
    let candidates = resolve_paths("@/app/utils", &config);
    assert_eq!(candidates, vec!["src/app/utils"]);

    let candidates = resolve_paths("@lib/helper", &config);
    assert_eq!(candidates, vec!["lib/helper", "shared/helper"]);
}

#[test]
fn ts_extension_candidates_test() {
    let c = ts_extension_candidates(".js");
    assert!(c.contains(&".ts"));
    assert!(c.contains(&".tsx"));
    assert!(c.contains(&".d.ts"));

    let c = ts_extension_candidates("");
    assert!(c.contains(&".ts"));
    assert!(c.contains(&"/index.ts"));

    let c = ts_extension_candidates(".mjs");
    assert!(c.contains(&".mts"));
    assert!(c.contains(&".d.mts"));
}
