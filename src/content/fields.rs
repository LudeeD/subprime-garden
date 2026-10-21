//! Typed collection fields: checks values against a collection's schema
//! (`[[collections]] fields` in garden.toml). Every path that writes fields —
//! the admin form, import, and the startup content check — goes through
//! `validate`, so an unknown field or a wrong type is always an error, never
//! silently dropped.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::config::FieldType;

pub type Schema = BTreeMap<String, FieldType>;

/// Checks `input` against `schema` and returns the values to store: nulls
/// dropped (an empty field is absent, not null) and dates normalized.
/// `media_exists` answers whether a filename is in the media store.
pub fn validate(
    schema: &Schema,
    input: &Map<String, Value>,
    media_exists: &dyn Fn(&str) -> bool,
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (name, value) in input {
        if value.is_null() {
            continue;
        }
        let Some(ty) = schema.get(name) else {
            return Err(format!("unknown field `{name}`"));
        };
        let value = check(ty, value, media_exists).map_err(|expected| {
            format!("field `{name}` must be {expected}, got {value}")
        })?;
        out.insert(name.clone(), value);
    }
    Ok(out)
}

/// The admin form's `field_<name>` inputs, which all arrive as strings.
/// Blank inputs are left out; ints are parsed so `validate` sees the same
/// types frontmatter would give it.
pub fn from_form<'a>(
    schema: &Schema,
    inputs: impl Iterator<Item = (&'a str, &'a str)>,
    media_exists: &dyn Fn(&str) -> bool,
) -> Result<Map<String, Value>, String> {
    let mut raw = Map::new();
    for (name, input) in inputs {
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        let value = match schema.get(name) {
            Some(FieldType::Int) => match input.parse::<i64>() {
                Ok(n) => Value::from(n),
                Err(_) => return Err(format!("field `{name}` must be a whole number, got {input:?}")),
            },
            _ => Value::from(input),
        };
        raw.insert(name.to_string(), value);
    }
    validate(schema, &raw, media_exists)
}

/// A stored value as an admin form input's `value` attribute.
pub fn form_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Returns the value to store, or a description of what was expected.
fn check(ty: &FieldType, value: &Value, media_exists: &dyn Fn(&str) -> bool) -> Result<Value, String> {
    match (ty, value) {
        (FieldType::Text, Value::String(_)) => Ok(value.clone()),
        (FieldType::Int, Value::Number(n)) if n.is_i64() => Ok(value.clone()),
        (FieldType::Date, Value::String(s)) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(|d| Value::from(d.format("%Y-%m-%d").to_string()))
            .map_err(|_| "a date (YYYY-MM-DD)".to_string()),
        (FieldType::Url, Value::String(s)) if is_absolute_url(s) => Ok(value.clone()),
        (FieldType::Media, Value::String(s)) if media_exists(s) => Ok(value.clone()),
        (FieldType::Choice(options), Value::String(s)) if options.contains(s) => Ok(value.clone()),
        _ => Err(expected(ty)),
    }
}

fn expected(ty: &FieldType) -> String {
    match ty {
        FieldType::Text => "text".into(),
        FieldType::Int => "a whole number".into(),
        FieldType::Date => "a date (YYYY-MM-DD)".into(),
        FieldType::Url => "an absolute http(s) URL".into(),
        FieldType::Media => "the filename of an uploaded media file".into(),
        FieldType::Choice(options) => format!("one of {}", options.join(", ")),
    }
}

fn is_absolute_url(s: &str) -> bool {
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/') && !s.contains(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn books() -> Schema {
        crate::config::test_collections().pop().unwrap().fields
    }

    fn check_one(field: &str, value: Value) -> Result<Map<String, Value>, String> {
        let input = Map::from_iter([(field.to_string(), value)]);
        validate(&books(), &input, &|name| name == "cover.png")
    }

    #[test]
    fn accepts_every_type() {
        let input = json!({
            "author": "Ursula K. Le Guin",
            "rating": 5,
            "finished": "2026-09-14",
            "link": "https://example.com/book",
            "cover": "cover.png",
            "shelf": "read",
        });
        let stored = validate(&books(), input.as_object().unwrap(), &|_| true).unwrap();
        assert_eq!(Value::Object(stored), input);
    }

    #[test]
    fn rejects_unknown_fields_and_wrong_types() {
        assert!(check_one("isbn", json!("123")).unwrap_err().contains("unknown field"));
        assert!(check_one("rating", json!("5")).is_err());
        assert!(check_one("rating", json!(4.5)).is_err());
        assert!(check_one("finished", json!("14/09/2026")).is_err());
        assert!(check_one("link", json!("example.com")).is_err());
        assert!(check_one("cover", json!("missing.png")).is_err());
        assert!(check_one("shelf", json!("lost")).is_err());
    }

    #[test]
    fn nulls_and_blank_form_inputs_are_left_out() {
        assert!(check_one("author", Value::Null).unwrap().is_empty());
        let form = [("author", " "), ("rating", "4")];
        let stored = from_form(&books(), form.into_iter(), &|_| true).unwrap();
        assert_eq!(Value::Object(stored), json!({"rating": 4}));
    }
}
