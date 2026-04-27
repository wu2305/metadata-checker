use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::types::RefType;

/// 表达式引用提取正则（编译一次，全局复用）
static EXPR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[\p{L}@$_][\p{L}\p{N}_]*(?:\.[\p{L}\p{N}_]+)+|\$[\p{L}\p{N}_]+|\bparam[\p{L}\p{N}_]*\b|\bmodel[\p{L}\p{N}_]*\b",
    )
    .unwrap()
});

pub fn parse_expression_refs(expr: &str) -> Vec<RefType> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();

    for mat in EXPR_RE.find_iter(expr) {
        let token = mat.as_str();

        let lower = token.to_lowercase();
        if [
            "if",
            "null",
            "true",
            "false",
            "undefined",
            "and",
            "or",
            "not",
            "is",
            "in",
            "between",
        ]
        .contains(&lower.as_str())
        {
            continue;
        }
        if token.parse::<f64>().is_ok() {
            continue;
        }
        if token.starts_with('\'') || token.starts_with('"') {
            continue;
        }
        if seen.contains(token) {
            continue;
        }

        if is_in_string_literal(expr, mat.start()) {
            continue;
        }

        seen.insert(token.to_string());
        let ref_type = classify_ref(token);
        refs.push(ref_type);
    }

    refs
}

/// 判断 token 是否位于字符串字面量内部（支持转义引号）
fn is_in_string_literal(expr: &str, pos: usize) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < pos && i < bytes.len() {
        let c = bytes[i] as char;
        if c == '\\' {
            i += 1; // skip next character (escaped)
        } else if c == (39 as char) && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        }
        i += 1;
    }

    in_single_quote || in_double_quote
}

/// 根据 token 特征分类引用类型
fn classify_ref(token: &str) -> RefType {
    let token = token.trim();

    if token.starts_with('$') {
        let parts: Vec<&str> = token[1..].split('.').collect();
        if parts.len() >= 2 {
            return RefType::UserProperty(token[1..].to_string());
        }
        return RefType::SystemVar(token.to_string());
    }

    let parts: Vec<&str> = token.split('.').collect();

    if parts.len() >= 2 {
        let first = parts[0];
        if first.starts_with("model") {
            let field = parts[1..].join(".");
            return RefType::ModelField(first.to_string(), field);
        }
        if parts.last() == Some(&"value") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.last() == Some(&"step") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.len() >= 3 && parts[1] == "checked" && parts[2] == "value" {
            return RefType::ComponentProperty(first.to_string(), "checked.value".to_string());
        }
        return RefType::Other(token.to_string());
    }

    if token.starts_with("param") {
        return RefType::Param(token.to_string());
    }

    if !token
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true)
    {
        return RefType::ComponentValue(token.to_string());
    }

    RefType::Other(token.to_string())
}
