use super::*;

#[test]
fn extracts_static_side_effect_reexports_and_types_with_spans() {
    let source = "import React, {useState as use} from 'react';\nimport './polyfill';\nexport {x} from './x';\nexport type {T} from './types';\nimport type {U} from './u';";
    let refs = extract_typescript_imports_structured(source);
    assert_eq!(refs.len(), 5);
    assert_eq!(refs[0].specifier, "react");
    assert_eq!(refs[0].kind, ImportKind::Static);
    assert_eq!(refs[1].kind, ImportKind::SideEffect);
    assert_eq!(refs[2].kind, ImportKind::ReExport);
    assert_eq!(refs[3].kind, ImportKind::TypeOnly);
    assert_eq!(refs[4].kind, ImportKind::TypeOnly);
    assert_eq!(
        &source[refs[0].span.unwrap().start..refs[0].span.unwrap().end],
        "import React, {useState as use} from 'react';"
    );
}

#[test]
fn extracts_dynamic_import_forms_without_dropping_computed_syntax() {
    let refs = extract_typescript_imports_structured(
        "const a = import('./literal'); const b = import(`./${name}`);",
    );
    assert_eq!(refs.len(), 2);
    assert_eq!(
        (refs[0].specifier.as_str(), refs[0].kind),
        ("./literal", ImportKind::Static)
    );
    assert_eq!(refs[1].kind, ImportKind::Dynamic);
    assert_eq!(refs[1].specifier, "`./${name}`");
}

#[test]
fn extracts_import_equals_and_import_type_expression() {
    let refs = extract_typescript_imports_structured(
        "import fs = require('fs'); type Mod = import('./model').Model;",
    );
    assert_eq!(refs.len(), 2);
    assert_eq!(
        (refs[0].specifier.as_str(), refs[0].kind),
        ("fs", ImportKind::ModuleRequires)
    );
    assert_eq!(
        (refs[1].specifier.as_str(), refs[1].kind),
        ("./model", ImportKind::Static)
    );
}

#[test]
fn extracts_literal_and_computed_require_explicitly() {
    let refs = extract_imports_structured(
        "const a = require('./literal'); const b = require(prefix + name);",
    );
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].kind, ImportKind::ModuleRequires);
    assert_eq!(refs[1].kind, ImportKind::Dynamic);
    assert_eq!(refs[1].specifier, "prefix + name");
}

#[test]
fn extracts_tsx_and_survives_invalid_input() {
    assert_eq!(
        extract_tsx_imports("import {Button} from './Button'; const a = <Button/>;"),
        vec!["./Button"]
    );
    let _ = extract_imports("import {{{ broken 'unclosed");
}
