//! Claims that say the same thing twice. A claim is near another when it is about the same host and
//! class and their names and summaries share most of their words. The shelf warns the publisher
//! and the clerk raises it; nothing is merged or dropped.

use serde_json::Value;

fn words(claim: &Value) -> std::collections::BTreeSet<String> {
    let text = format!("{} {}", claim.get("name").and_then(Value::as_str).unwrap_or(""), claim.get("summary").and_then(Value::as_str).unwrap_or(""));
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4).map(str::to_string).collect()
}

fn field(claim: &Value, name: &str) -> String {
    claim.get(name).and_then(Value::as_str).unwrap_or("").trim().to_lowercase()
}

/// Whether two claim records are about the same thing.
pub fn near(a: &Value, b: &Value) -> bool {
    if field(a, "host") != field(b, "host") || field(a, "vulnclass") != field(b, "vulnclass") {
        return false;
    }
    let (wa, wb) = (words(a), words(b));
    if wa.len() < 4 || wb.len() < 4 {
        return false;
    }
    let shared = wa.intersection(&wb).count() as f64;
    let all = wa.union(&wb).count() as f64;
    shared / all >= 0.6
}
