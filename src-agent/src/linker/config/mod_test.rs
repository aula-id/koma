use super::*;

#[test]
fn tokenize_command_basic() {
    let tokens = tokenize_command("gcc -I/usr/include -o out file.c");
    assert_eq!(tokens, vec!["gcc", "-I/usr/include", "-o", "out", "file.c"]);
}

#[test]
fn tokenize_command_quoted() {
    let tokens = tokenize_command("gcc '-I/usr/my include' file.c");
    assert_eq!(tokens, vec!["gcc", "-I/usr/my include", "file.c"]);
}

#[test]
fn compile_flags_parse_i() {
    let content = "-I/usr/include\n-I./src\n";
    let base = "/proj";
    // Simulate what parse_compile_flags does
    let mut flags = CompileFlags::default();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("-I") {
            let rest = &line[2..];
            flags.include_paths.push(resolve_rel(base, rest));
        }
    }
    assert_eq!(flags.include_paths.len(), 2);
    assert_eq!(flags.include_paths[0], "/usr/include");
    assert_eq!(flags.include_paths[1], "/proj/src");
}

#[test]
fn compile_db_from_json() {
    let json = r#"[
        {
            "directory": "/proj/build",
            "file": "../src/main.c",
            "arguments": ["gcc", "-I", "/usr/include", "-I", "./include", "-x", "c", "../src/main.c"]
        }
    ]"#;
    let path = Path::new("/proj/build/compile_commands.json");
    let db = CompileDB::parse_json(json, path).unwrap();
    assert_eq!(db.entries.len(), 1);
    let entry = db.lookup("/proj/src/main.c").unwrap();
    assert_eq!(entry.directory, "/proj/build");
    let flags = entry.extract_flags();
    assert_eq!(flags.include_paths.len(), 2);
    assert_eq!(flags.language_mode.as_deref(), Some("c"));
}

#[test]
fn compile_db_command_fallback() {
    let json = r#"[
        {
            "directory": "/proj",
            "file": "src/main.c",
            "command": "gcc -I/usr/include -I./src src/main.c"
        }
    ]"#;
    let path = Path::new("/proj/compile_commands.json");
    let db = CompileDB::parse_json(json, path).unwrap();
    let entry = db.lookup("/proj/src/main.c").unwrap();
    let flags = entry.extract_flags();
    assert_eq!(flags.include_paths.len(), 2);
}

#[test]
fn compile_flags_txt_parse() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-linker-test-flags-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("compile_flags.txt"),
        "-I/usr/include\n-I./include\n-x\nc++\n",
    )
    .unwrap();
    let flags = parse_compile_flags(&tmp.join("compile_flags.txt"));
    assert_eq!(flags.include_paths.len(), 2);
    assert_eq!(flags.language_mode.as_deref(), Some("c++"));
    let _ = std::fs::remove_dir_all(&tmp);
}
