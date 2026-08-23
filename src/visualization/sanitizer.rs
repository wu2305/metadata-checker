use serde_json::{Map, Value};
use std::hash::Hasher;
use twox_hash::XxHash64;

use crate::output::schema::{Diagnostic, Location};

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
        .replace_all(&result, |captures: &regex::Captures| {
            let key = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            let separator = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
            let value = captures.get(3).map(|m| m.as_str()).unwrap_or_default();
            if is_internal_redacted_hash(value) {
                captures
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                format!("{}{}{}", key, separator, REDACTED_VALUE)
            }
        })
        .to_string();

    result
}

/// 清洗图身份 ID，并在发生脱敏时追加稳定短 hash 防止 ID 碰撞。
pub fn sanitize_identity_id(raw_id: &str) -> String {
    let safe_id = sanitize_text(raw_id);
    if safe_id == raw_id {
        safe_id
    } else {
        format!("{}__h{}", safe_id, short_hash(raw_id))
    }
}

/// 计算不可逆短 hash，用于区分脱敏后文本相同的原始 ID。
fn short_hash(text: &str) -> String {
    let mut hasher = XxHash64::default();
    hasher.write(text.as_bytes());
    format!("{:08x}", (hasher.finish() & 0xffff_ffff) as u32)
}

/// 判断是否为内部生成的已脱敏 identity hash 后缀。
fn is_internal_redacted_hash(value: &str) -> bool {
    value.len() == 14
        && value.starts_with("***__h")
        && value[6..].chars().all(|c| c.is_ascii_hexdigit())
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
        count: diagnostic.count,
        answer_impact: diagnostic.answer_impact.clone(),
        first_seen_phase: diagnostic.first_seen_phase.clone(),
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
