use crate::output::schema::{Diagnostic, Location};
use serde_json::{Map, Value};

/// 敏感字段关键词（匹配 key=value / key: value 这类写法）
const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "password",
    "secret",
    "cookie",
    "auth",
    "credential",
    "api_key",
    "apikey",
];

/// 敏感值统一替换占位符
const REDACTED_VALUE: &str = "***";

/// 将包含敏感键值对的文本脱敏。
pub fn sanitize_text(text: &str) -> String {
    let mut result = text.to_string();
    let pattern = SENSITIVE_KEYS.join("|");
    let pair_regex = regex::Regex::new(&format!(
        r#"(?i)({})(\s*[:=]\s*)("[^"]*"|'[^']*'|`[^`]*`|[^,\s;\)\]]+)"#,
        pattern
    ))
    .expect("invalid sensitive pair regex");

    result = pair_regex
        .replace_all(&result, format!("$1$2{}", REDACTED_VALUE))
        .to_string();

    result
}

/// 判断字段名是否为敏感字段名。
pub fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEYS.iter().any(|s| lower.contains(s))
}

/// 递归清洗 JSON 值，保证 metadata 里的字符串不会带出敏感明文。
pub fn sanitize_metadata_value(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(sanitize_text(value)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(sanitize_metadata_value)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            let mut sanitized = Map::new();
            for (key, value) in map {
                sanitized.insert(key.clone(), sanitize_metadata_entry(key, value));
            }
            Value::Object(sanitized)
        }
        other => other.clone(),
    }
}

/// 按 JSON key 语义清洗 metadata 条目。
pub fn sanitize_metadata_entry(key: &str, value: &Value) -> Value {
    if is_sensitive_key(key) {
        Value::String(REDACTED_VALUE.to_string())
    } else {
        sanitize_metadata_value(value)
    }
}

/// 清洗可视化诊断信息。
pub fn sanitize_diagnostic(diagnostic: &Diagnostic) -> Diagnostic {
    Diagnostic {
        severity: diagnostic.severity.clone(),
        code: sanitize_text(&diagnostic.code),
        message: sanitize_text(&diagnostic.message),
        location: sanitize_location(&diagnostic.location),
        suggestion: diagnostic
            .suggestion
            .as_ref()
            .map(|text| sanitize_text(text)),
    }
}

/// 清洗诊断位置信息。
pub fn sanitize_location(location: &Location) -> Location {
    Location {
        source_file: location
            .source_file
            .as_ref()
            .map(|source_file| sanitize_text(source_file)),
        node_id: location
            .node_id
            .as_ref()
            .map(|node_id| sanitize_text(node_id)),
        json_path: location
            .json_path
            .as_ref()
            .map(|json_path| sanitize_text(json_path)),
    }
}
