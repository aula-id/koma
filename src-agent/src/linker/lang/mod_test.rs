use super::*;

#[test]
fn detect_lang_works() {
    assert_eq!(detect_lang("foo.rs"), Lang::Rust);
    assert_eq!(detect_lang("bar.py"), Lang::Python);
    assert_eq!(detect_lang("baz.go"), Lang::Go);
    assert_eq!(detect_lang("Qux.java"), Lang::Java);
    assert_eq!(detect_lang("mod.ts"), Lang::TypeScript);
    assert_eq!(detect_lang("mod.tsx"), Lang::TypeScript);
    assert_eq!(detect_lang("app.js"), Lang::JavaScript);
    assert_eq!(detect_lang("app.jsx"), Lang::JavaScript);
    assert_eq!(detect_lang("app.mjs"), Lang::JavaScript);
    assert_eq!(detect_lang("app.cjs"), Lang::JavaScript);
    assert_eq!(detect_lang("index.php"), Lang::Php);
    assert_eq!(detect_lang("main.c"), Lang::C);
    assert_eq!(detect_lang("header.h"), Lang::C);
    assert_eq!(detect_lang("app.cpp"), Lang::Cpp);
    assert_eq!(detect_lang("app.cc"), Lang::Cpp);
    assert_eq!(detect_lang("app.cxx"), Lang::Cpp);
    assert_eq!(detect_lang("app.hpp"), Lang::Cpp);
    assert_eq!(detect_lang("app.hxx"), Lang::Cpp);
    assert_eq!(detect_lang("app.hh"), Lang::Cpp);
    assert_eq!(detect_lang("main.dart"), Lang::Dart);
    assert_eq!(detect_lang("App.swift"), Lang::Swift);
    assert_eq!(detect_lang("foo.txt"), Lang::Unknown);
}
