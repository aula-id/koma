//! Language-specific import extractors and detection.
//!
//! Each extractor parses source code with tree-sitter and returns a list of raw
//! import/module strings. Extraction is best-effort: parse failures yield empty
//! results, never panics.

pub mod go_lang;
pub mod java;
pub mod php;
pub mod python;
pub mod rust;
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
    } else {
        Lang::Unknown
    }
}

/// File extensions the scanner cares about.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".go", ".java", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".php",
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
        assert_eq!(detect_lang("foo.txt"), Lang::Unknown);
    }
}
