#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

// -------------------------------------------------------------------------
// parse_function_param_call
// -------------------------------------------------------------------------

#[test]
fn harmony_single_param_string() {
    let inner = "<function=greet>\n<parameter=name>Alice\n</function>";
    let (name, args) = parse_function_param_call(inner).expect("should parse");
    assert_eq!(name, "greet");
    let v: serde_json::Value = serde_json::from_str(&args).unwrap();
    assert_eq!(v["name"], serde_json::Value::String("Alice".to_string()));
}

#[test]
fn harmony_multi_param_type_coercion() {
    // port should become a number, action a string, enabled a bool
    let inner = "<function=sec_remote>\n<parameter=action>open\n<parameter=host>localhost\n<parameter=port>3000\n<parameter=enabled>true\n</function>";
    let (name, args) = parse_function_param_call(inner).expect("should parse");
    assert_eq!(name, "sec_remote");
    let v: serde_json::Value = serde_json::from_str(&args).unwrap();
    assert_eq!(v["action"], serde_json::Value::String("open".to_string()));
    assert_eq!(
        v["host"],
        serde_json::Value::String("localhost".to_string())
    );
    assert_eq!(v["port"], serde_json::json!(3000));
    assert_eq!(v["enabled"], serde_json::json!(true));
}

#[test]
fn harmony_no_params_returns_empty_object() {
    let inner = "<function=ping>";
    let (name, args) = parse_function_param_call(inner).expect("should parse");
    assert_eq!(name, "ping");
    assert_eq!(args, "{}");
}

#[test]
fn harmony_no_params_with_close_tag() {
    let inner = "<function=ping></function>";
    let (name, args) = parse_function_param_call(inner).expect("should parse");
    assert_eq!(name, "ping");
    assert_eq!(args, "{}");
}

#[test]
fn harmony_param_with_close_tags() {
    let inner = "<function=tool>\n<parameter=key>value</parameter>\n</function>";
    let (name, args) = parse_function_param_call(inner).expect("should parse");
    assert_eq!(name, "tool");
    let v: serde_json::Value = serde_json::from_str(&args).unwrap();
    assert_eq!(v["key"], serde_json::Value::String("value".to_string()));
}

#[test]
fn harmony_empty_name_returns_none() {
    let inner = "<function=></function>";
    assert!(parse_function_param_call(inner).is_none());
}

#[test]
fn harmony_missing_function_tag_returns_none() {
    let inner = "<parameter=key>value";
    assert!(parse_function_param_call(inner).is_none());
}

// -------------------------------------------------------------------------
// extract_text_tool_calls — wrapped (mimo / <tool_call> outer)
// -------------------------------------------------------------------------

#[test]
fn wrapped_harmony_single_param() {
    let content = "<tool_call>\n<function=sec_remote>\n<parameter=action>open\n</tool_call>";
    let (cleaned, calls) = extract_text_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "sec_remote");
    let v: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(v["action"], serde_json::Value::String("open".to_string()));
    // span must be removed
    assert!(!cleaned.contains("<tool_call>"));
    assert!(!cleaned.contains("<function="));
}

#[test]
fn wrapped_harmony_multi_param_coercion() {
    let content = "Before\n<tool_call>\n<function=sec_remote>\n<parameter=action>open\n<parameter=host>localhost\n<parameter=port>3000\n</tool_call>\nAfter";
    let (cleaned, calls) = extract_text_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "sec_remote");
    let v: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(v["port"], serde_json::json!(3000));
    assert_eq!(v["action"], serde_json::Value::String("open".to_string()));
    assert!(cleaned.contains("Before"));
    assert!(cleaned.contains("After"));
    assert!(!cleaned.contains("<tool_call>"));
}

#[test]
fn wrapped_json_form_still_works_regression() {
    let content = r#"<tool_call>{"name":"ls","arguments":{"path":"/tmp"}}</tool_call>"#;
    let (cleaned, calls) = extract_text_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "ls");
    let v: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(v["path"], serde_json::Value::String("/tmp".to_string()));
    assert!(cleaned.is_empty() || !cleaned.contains("<tool_call>"));
}

// -------------------------------------------------------------------------
// extract_text_tool_calls — standalone (no <tool_call> wrapper)
// -------------------------------------------------------------------------

#[test]
fn standalone_harmony_call() {
    let content =
        "Here is the call:\n<function=say_hi>\n<parameter=name>Bob\n</function>\nDone.";
    let (cleaned, calls) = extract_text_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, "say_hi");
    let v: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(v["name"], serde_json::Value::String("Bob".to_string()));
    assert!(cleaned.contains("Here is the call:"));
    assert!(cleaned.contains("Done."));
    assert!(!cleaned.contains("<function="));
}

// -------------------------------------------------------------------------
// span removal — no markup leaks into cleaned content
// -------------------------------------------------------------------------

#[test]
fn no_markup_leak_in_cleaned_content() {
    let content =
        "Prose\n<tool_call>\n<function=tool>\n<parameter=x>1\n</tool_call>\nMore prose";
    let (cleaned, calls) = extract_text_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert!(!cleaned.contains("<tool_call>"));
    assert!(!cleaned.contains("</tool_call>"));
    assert!(!cleaned.contains("<function="));
    assert!(!cleaned.contains("<parameter="));
    assert!(cleaned.contains("Prose"));
    assert!(cleaned.contains("More prose"));
}

// -------------------------------------------------------------------------
// strip_tool_call_tags — harmony orphan tag hygiene
// -------------------------------------------------------------------------

#[test]
fn strip_removes_orphan_harmony_tags() {
    let content = "Hello <function=foo><parameter=bar>val</parameter></function> world";
    let stripped = strip_tool_call_tags(content);
    assert!(!stripped.contains("<function="));
    assert!(!stripped.contains("<parameter="));
    assert!(!stripped.contains("</function>"));
    assert!(!stripped.contains("</parameter>"));
    assert!(stripped.contains("Hello"));
    assert!(stripped.contains("world"));
}

#[test]
fn strip_leaves_prose_intact() {
    let content = "No tags here.";
    assert_eq!(strip_tool_call_tags(content), "No tags here.");
}

#[test]
fn strip_removes_plural_tool_calls_tags() {
    let content = "[summary of earlier conversation]\n<tool_calls>\n</tool_calls>";
    let stripped = strip_tool_call_tags(content);
    assert!(!stripped.contains("<tool_calls>"));
    assert!(!stripped.contains("</tool_calls>"));
    assert!(stripped.contains("[summary of earlier conversation]"));
}

#[test]
fn strip_removes_orphan_plural_open_tag() {
    let content = "Some text\n<tool_calls>\ntrailing";
    let stripped = strip_tool_call_tags(content);
    assert!(!stripped.contains("<tool_calls>"));
}

#[test]
fn strip_removes_orphan_plural_close_tag() {
    let content = "Some text\n</tool_calls>\nmore";
    let stripped = strip_tool_call_tags(content);
    assert!(!stripped.contains("</tool_calls>"));
    assert!(stripped.contains("Some text"));
}
