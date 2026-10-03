//! 文档漂移防护：`docs/reference/schema.md` 顶层结构里的 `kind` 枚举
//! 必须与代码实际会序列化出来的 `OutputKind` 完全一致。
//!
//! 背景：`QueryAdvice` 加进枚举后一直没进文档（schema.md 的 kind 行），
//! 没有任何测试能发现。本测试让两边任何一侧单独变动都会失败。

use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use metadata_checker::output::OutputKind;

/// 枚举全部变体。
///
/// `match` 故意不写通配分支：新增变体时这里会编译失败，
/// 迫使维护者同步更新 `all_output_kinds`，再由下面的测试要求同步文档。
fn exhaustive_check(kind: &OutputKind) {
    match kind {
        OutputKind::SuperPage
        | OutputKind::PageQuery
        | OutputKind::ModelQuery
        | OutputKind::CrossPageQuery
        | OutputKind::DataFlowQuery
        | OutputKind::ComponentQuery
        | OutputKind::PriorityQuery
        | OutputKind::Explain
        | OutputKind::Context
        | OutputKind::PageLogic
        | OutputKind::Table
        | OutputKind::DataFlow
        | OutputKind::GraphDbCheck
        | OutputKind::QueryAdvice => {}
    }
}

/// 代码侧事实：每个变体经 serde 序列化后的线上字符串。
fn emitted_kinds() -> Result<BTreeSet<String>> {
    let all = [
        OutputKind::SuperPage,
        OutputKind::PageQuery,
        OutputKind::ModelQuery,
        OutputKind::CrossPageQuery,
        OutputKind::DataFlowQuery,
        OutputKind::ComponentQuery,
        OutputKind::PriorityQuery,
        OutputKind::Explain,
        OutputKind::Context,
        OutputKind::PageLogic,
        OutputKind::Table,
        OutputKind::DataFlow,
        OutputKind::GraphDbCheck,
        OutputKind::QueryAdvice,
    ];
    all.iter()
        .map(|kind| {
            exhaustive_check(kind);
            let value = serde_json::to_value(kind).context("序列化 OutputKind 失败")?;
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("OutputKind 应序列化为字符串，实际为 {value}"))
        })
        .collect()
}

/// 文档侧事实：顶层结构 JSON 示例里 `"kind": "A | B | ..."` 这一行的取值集合。
fn documented_kinds() -> Result<BTreeSet<String>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/reference/schema.md");
    let text = std::fs::read_to_string(path).with_context(|| format!("读取 {path} 失败"))?;
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("\"kind\": \""))
        .ok_or_else(|| anyhow!("schema.md 中找不到顶层结构的 \"kind\" 枚举行"))?;
    let start = line
        .find(": \"")
        .map(|idx| idx + 3)
        .context("kind 行缺少起始引号")?;
    let end = line
        .rfind('"')
        .filter(|end| *end > start)
        .context("kind 行缺少结束引号")?;
    Ok(line[start..end]
        .split('|')
        .map(|name| name.trim().to_owned())
        .collect())
}

/// 文档的 kind 枚举与代码的 `OutputKind` 必须一一对应，缺失和多余都要报。
#[test]
fn schema_doc_kind_list_matches_output_kind() -> Result<()> {
    let emitted = emitted_kinds()?;
    let documented = documented_kinds()?;
    let missing_in_doc: Vec<_> = emitted.difference(&documented).collect();
    let unknown_in_doc: Vec<_> = documented.difference(&emitted).collect();
    assert_eq!(
        (missing_in_doc, unknown_in_doc),
        (Vec::<&String>::new(), Vec::<&String>::new()),
        "docs/reference/schema.md 的 kind 枚举与 OutputKind 不一致：\
         左边是代码有而文档缺的，右边是文档有而代码没有的"
    );
    Ok(())
}
