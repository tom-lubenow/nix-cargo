use std::fmt::Write;

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn shell_array_literal(values: &[String]) -> String {
    if values.is_empty() {
        return String::new();
    }

    values
        .iter()
        .map(|value| shell_single_quote(value))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn nix_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return String::from("[ ]");
    }

    let mut result = String::from("[");
    for value in values {
        let _ = write!(result, " \"{}\"", nix_escape(value));
    }
    result.push_str(" ]");
    result
}

pub(crate) fn nix_optional_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", nix_escape(value)))
        .unwrap_or_else(|| String::from("null"))
}

pub(crate) fn nix_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

pub(crate) fn nix_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
}

