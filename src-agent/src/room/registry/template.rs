//! Prompt templates: plain text with `{{var}}` placeholders.

/// One piece of a parsed template.
#[derive(Debug, PartialEq)]
enum Segment<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn valid_var(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn scan(body: &str) -> Result<Vec<Segment<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find("{{") {
        if open > 0 {
            out.push(Segment::Text(&rest[..open]));
        }
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err("unclosed '{{' placeholder".to_string());
        };
        let name = after[..close].trim();
        if !valid_var(name) {
            return Err(format!("bad placeholder name '{name}'"));
        }
        out.push(Segment::Var(name));
        rest = &after[close + 2..];
    }
    if !rest.is_empty() {
        out.push(Segment::Text(rest));
    }
    Ok(out)
}

/// Check that a template parses. Used at load time so a broken file is
/// rejected before it can reach a turn.
pub fn validate(body: &str) -> Result<(), String> {
    scan(body).map(|_| ())
}

/// Why a template could not be filled.
#[derive(Debug, PartialEq)]
pub enum TemplateError {
    /// The template text itself is malformed.
    Syntax(String),
    /// A placeholder has no value in the supplied variables.
    MissingVar(String),
}

/// Fill `{{var}}` placeholders from `vars`. Extra variables are ignored.
pub fn render(body: &str, vars: &[(&str, &str)]) -> Result<String, TemplateError> {
    let segments = scan(body).map_err(TemplateError::Syntax)?;
    let mut out = String::with_capacity(body.len());
    for segment in segments {
        match segment {
            Segment::Text(text) => out.push_str(text),
            Segment::Var(name) => match vars.iter().find(|(k, _)| *k == name) {
                Some((_, value)) => out.push_str(value),
                None => return Err(TemplateError::MissingVar(name.to_string())),
            },
        }
    }
    Ok(out)
}
