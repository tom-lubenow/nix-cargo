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
