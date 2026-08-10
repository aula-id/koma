//! Language-specific import extractors and detection.
//!
//! Each extractor parses source code with tree-sitter and returns a list of raw
//! import/module strings. Extraction is best-effort: parse failures yield empty
//! results, never panics.

pub mod c_lang;
pub mod cpp_lang;
pub mod cpp_shared;
pub mod dart;
pub mod go_lang;
pub mod java;
pub mod php;
pub mod python;
pub mod rust;
pub mod swift;
pub mod typescript;

use super::graph::Lang;

/// Detect language from file extension.
pub fn detect_lang(path: &str) -> Lang {
    if path.ends_with(".rs") {
        Lang::Rust
    } else if path.ends_with(".py") {
        Lang::Python
    } else if path.ends_with(".go") {
        Lang::Go
    } else if path.ends_with(".java") {
        Lang::Java
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        Lang::TypeScript
    } else if path.ends_with(".js")
        || path.ends_with(".jsx")
        || path.ends_with(".mjs")
        || path.ends_with(".cjs")
    {
        Lang::JavaScript
    } else if path.ends_with(".php") {
        Lang::Php
    } else if path.ends_with(".c") || path.ends_with(".h") {
        Lang::C
    } else if path.ends_with(".cpp")
        || path.ends_with(".cc")
        || path.ends_with(".cxx")
        || path.ends_with(".hpp")
        || path.ends_with(".hxx")
    {
        Lang::Cpp
    } else if path.ends_with(".dart") {
        Lang::Dart
    } else if path.ends_with(".swift") {
        Lang::Swift
    } else {
        Lang::Unknown
    }
}

/// File extensions the scanner cares about.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    ".rs",
    ".py",
    ".go",
    ".java",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".mjs",
    ".cjs",
    ".php",
    ".c",
    ".h",
    ".cpp",
    ".cc",
    ".cxx",
    ".hpp",
    ".hxx",
    ".dart",
    ".swift",
];

/// Dispatch to the correct language extractor.
pub fn extract_imports(lang: Lang, content: &str) -> Vec<String> {
    match lang {
        Lang::Rust => rust::extract_imports(content),
        Lang::Python => python::extract_imports(content),
        Lang::Go => go_lang::extract_imports(content),
        Lang::Java => java::extract_imports(content),
        Lang::TypeScript | Lang::JavaScript => typescript::extract_imports(content),
        Lang::Php => php::extract_imports(content),
        Lang::C => c_lang::extract_imports(content),
        Lang::Cpp => cpp_lang::extract_imports(content),
        Lang::Dart => dart::extract_imports(content),
        Lang::Swift => swift::extract_imports(content),
        Lang::Unknown => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(detect_lang("main.dart"), Lang::Dart);
        assert_eq!(detect_lang("App.swift"), Lang::Swift);
        assert_eq!(detect_lang("foo.txt"), Lang::Unknown);
    }
}
