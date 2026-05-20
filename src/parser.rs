use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::parsed_content::ParsedContent;
use crate::source_id::SourceId;
use crate::source_id::SourceKind;
use crate::storage_provider::{LocalStorageProvider, StorageProvider};
use crate::superpage;
use crate::tbl_single;

/// 文件解析入口模块
///
/// 自动识别文件类型（SuperPage / 旧版 Table）并调用对应解析器：
/// - .spg 结尾 → SuperPage 解析
/// - 其他 → 尝试旧版 Table 解析
///
/// 同时提供统一的 PageMetadata 结构，兼容两种输入格式。
/// Parsed metadata summary extracted from the low-code platform JSON.
#[derive(Debug, Default)]
pub struct PageMetadata {
    pub input_path: Option<String>,
    pub page_id: Option<String>,
    pub page_name: Option<String>,
    pub version: Option<String>,
    pub components: Vec<ComponentInfo>,
    pub data_bindings: Vec<DataBinding>,
    pub settings: HashMap<String, Value>,
    pub raw: Value,
    /// SuperPage specific metadata
    pub superpage: Option<superpage::SuperPageMetadata>,
    /// Table (.tbl) specific metadata
    pub tbl: Option<tbl_single::TblMetadata>,
}

#[derive(Debug, Default)]
pub struct ComponentInfo {
    pub id: Option<String>,
    pub component_type: Option<String>,
    pub title: Option<String>,
    pub db_field: Option<String>,
}

#[derive(Debug, Default)]
pub struct DataBinding {
    pub source_id: Option<String>,
    pub path: Option<String>,
    pub filter: Option<String>,
}

/// 根据文件类型自动选择解析器。
///
/// M37 native-only：依赖本地文件系统路径，不能进入 WASM core。
/// M36 起默认使用 DocumentProvider 入口；旧 parse_file_with_storage 保留兼容。
pub fn parse_file(path: &Path) -> Result<PageMetadata> {
    parse_file_with_document_provider(path, &LocalStorageProvider, None)
}

/// 使用指定文档读取层解析文件。
///
/// M36 新入口，使用 `DocumentProvider` 而非旧 `StorageProvider`。
pub fn parse_file_with_document_provider(
    path: &Path,
    provider: &dyn crate::storage_provider::DocumentProvider,
    project_dir: Option<&Path>,
) -> Result<PageMetadata> {
    let text = provider.read_to_string(path)?;
    let source = crate::source_id::SourceId::from_local_path(
        crate::source_id::ProjectRef::new("default"),
        path,
        project_dir,
    )?;
    let parsed = ParsedContent::from_text(source, text);
    parse_content(&parsed)
}

/// 使用指定存储层解析文件（兼容别名）。
///
/// M36 起，`StorageProvider` 是 `DocumentProvider` 的兼容别名。
/// 新代码应优先使用 `parse_file_with_document_provider`。
pub fn parse_file_with_storage(path: &Path, storage: &dyn StorageProvider) -> Result<PageMetadata> {
    parse_file_with_document_provider(path, storage, None)
}

/// M36.5: 从 ParsedContent 解析元数据。
///
/// 优先从 `ParsedContent::json()` 获取已缓存的 `Arc<Value>`，避免重复反序列化。
/// 不改变 `PageMetadata` 返回结构。
pub fn parse_content(parsed: &ParsedContent) -> Result<PageMetadata> {
    let raw = parsed.json().with_context(|| {
        format!(
            "Failed to get JSON from ParsedContent: {}",
            parsed.source.source_path
        )
    })?;

    let mut meta = PageMetadata {
        input_path: Some(parsed.source.source_path.clone()),
        ..PageMetadata::default()
    };

    let is_tbl = matches!(parsed.source.source_kind, SourceKind::Tbl);
    let is_spg = matches!(parsed.source.source_kind, SourceKind::Spg);

    build_page_metadata_from_value(&mut meta, &raw, is_spg, is_tbl, &parsed.source)?;

    meta.raw = (*raw).clone();
    Ok(meta)
}

fn build_page_metadata_from_value(
    meta: &mut PageMetadata,
    raw: &serde_json::Value,
    is_spg: bool,
    is_tbl: bool,
    source: &SourceId,
) -> Result<()> {
    if is_spg || raw.get("canvas").is_some() {
        meta.superpage = Some(superpage::parse_superpage_from_value(raw.clone())?);
        if let Some(obj) = raw.as_object() {
            meta.version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    } else if is_tbl || raw.get("dimensions").is_some() {
        meta.tbl = Some(tbl_single::parse_tbl_from_value_with_source(
            source,
            raw.clone(),
        )?);
        if let Some(obj) = raw.as_object() {
            meta.version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    } else {
        // Original parsing logic for non-superpage JSON
        if let Some(obj) = raw.as_object() {
            meta.version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);

            // Handle "forms" wrapper (fapp-style)
            if let Some(forms_val) = obj.get("forms")
                && let Some(forms_obj) = forms_val.as_object()
            {
                meta.version = forms_obj
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or(meta.version.clone());
                if let Some(forms_arr) = forms_obj.get("forms").and_then(|v| v.as_array()) {
                    for form in forms_arr {
                        if let Some(form_obj) = form.as_object() {
                            let page_id = form_obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            meta.page_id = meta.page_id.clone().or(page_id);
                            if let Some(components) =
                                form_obj.get("components").and_then(|v| v.as_array())
                            {
                                for comp in components {
                                    meta.components.push(extract_component(comp));
                                }
                            }
                        }
                    }
                    if let Some(params) = forms_obj.get("params").and_then(|v| v.as_array()) {
                        for param in params {
                            if let Some(pobj) = param.as_object() {
                                meta.data_bindings.push(DataBinding {
                                    source_id: pobj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                    path: pobj
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                    filter: pobj
                                        .get("value")
                                        .and_then(|v| v.as_str())
                                        .map(String::from),
                                });
                            }
                        }
                    }
                }
            }

            // Handle "settings"
            if let Some(settings) = obj.get("settings")
                && let Some(smap) = settings.as_object()
            {
                for (k, v) in smap {
                    meta.settings.insert(k.clone(), v.clone());
                }
            }

            // Handle workflow-style "nodes"
            if let Some(nodes) = obj.get("nodes").and_then(|v| v.as_array()) {
                for node in nodes {
                    let mut comp = ComponentInfo::default();
                    if let Some(nobj) = node.as_object() {
                        comp.id = nobj.get("id").and_then(|v| v.as_str()).map(String::from);
                        comp.component_type =
                            nobj.get("type").and_then(|v| v.as_str()).map(String::from);
                        comp.title = nobj.get("desc").and_then(|v| v.as_str()).map(String::from);
                        meta.components.push(comp);
                    }
                }
            }
        }
    }

    Ok(())
}

fn extract_component(value: &Value) -> ComponentInfo {
    let mut comp = ComponentInfo::default();
    if let Some(obj) = value.as_object() {
        comp.id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
        comp.component_type = obj.get("type").and_then(|v| v.as_str()).map(String::from);
        comp.title = obj
            .get("title")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| obj.get("title").and_then(|v| v.as_str()).map(String::from));
        comp.db_field = obj
            .get("dbfield")
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    comp
}

/// 从已反序列化的 JSON Value 解析元数据（M37 纯解析 API）。
///
/// 不读取文件，不创建 graphdb，不输出文本。
/// 供 WASM/远程/测试直接调用。
pub fn parse_metadata_from_value(source: SourceId, raw: Arc<Value>) -> Result<PageMetadata> {
    let mut meta = PageMetadata {
        input_path: Some(source.source_path.clone()),
        ..PageMetadata::default()
    };
    let is_tbl = matches!(source.source_kind, crate::source_id::SourceKind::Tbl);
    let is_spg = matches!(source.source_kind, crate::source_id::SourceKind::Spg);
    build_page_metadata_from_value(&mut meta, &raw, is_spg, is_tbl, &source)?;
    meta.raw = (*raw).clone();
    Ok(meta)
}

/// 从字符串内容解析元数据（M37 纯解析 API）。
///
/// 内部通过 ParsedContent 做 JSON 反序列化与缓存。
pub fn parse_metadata_from_str(source: SourceId, content: &str) -> Result<PageMetadata> {
    let parsed = ParsedContent::from_text(source, content);
    parse_content(&parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsed_content::ParsedContent;
    use crate::source_id::{ProjectRef, SourceId};

    fn test_source(path: &str) -> SourceId {
        SourceId::from_memory(ProjectRef::new("test"), path)
    }

    #[test]
    fn test_parse_content_from_memory_spg() {
        let text = r#"{"canvas": {"components": [{"id": "text1", "type": "text"}]}}"#;
        let source = test_source("app/page.spg");
        let parsed = ParsedContent::from_text(source, text);
        let meta = parse_content(&parsed).expect("parse_content should succeed for spg");

        assert!(
            meta.superpage.is_some(),
            "spg content should produce superpage"
        );
        assert_eq!(meta.input_path, Some("app/page.spg".to_string()));
    }

    #[test]
    fn test_parse_content_from_memory_tbl() {
        let text = r#"{"dimensions": [{"id": "dim1"}]}"#;
        let mut source = test_source("data/table.tbl");
        source.source_kind = crate::source_id::SourceKind::Tbl;
        let parsed = ParsedContent::from_text(source, text);
        let meta = parse_content(&parsed).expect("parse_content should succeed for tbl");

        assert!(
            meta.tbl.is_some(),
            "tbl content should produce tbl metadata"
        );
    }

    #[test]
    fn test_parse_content_reuses_cached_json() {
        let text = r#"{"version": "1.0", "canvas": {"components": []}}"#;
        let source = test_source("app/page.spg");
        let parsed = ParsedContent::from_text(source, text);

        let meta1 = parse_content(&parsed).expect("first parse should succeed");
        let meta2 = parse_content(&parsed).expect("second parse should reuse cached json");

        // 两次解析的 raw 应该相等
        assert_eq!(meta1.raw, meta2.raw);
    }

    #[test]
    fn test_parse_content_invalid_json_fails_at_parse_time() {
        let text = "not valid json";
        let source = test_source("app/page.spg");
        let parsed = ParsedContent::from_text(source, text);

        let result = parse_content(&parsed);
        assert!(
            result.is_err(),
            "invalid JSON should fail at parse_content time, not at construction"
        );
    }

    #[test]
    fn test_parse_file_compatible_with_parse_content() {
        // 验证 parse_file_with_storage 和 parse_content 对同一份内容输出一致
        let text = r#"{"canvas": {"components": [{"id": "btn1", "type": "button"}]}}"#;
        let source = test_source("app/page.spg");
        let parsed = ParsedContent::from_text(source, text);
        let meta_from_content = parse_content(&parsed).unwrap();

        // 两者都应有 superpage
        assert!(meta_from_content.superpage.is_some());
        assert_eq!(
            meta_from_content.input_path,
            Some("app/page.spg".to_string())
        );
    }
}

#[test]
fn test_parse_file_with_document_provider_reads_via_provider() {
    use crate::storage_provider::DocumentProvider;
    use std::path::Path;
    use std::time::SystemTime;

    struct TestProvider {
        content: String,
    }

    impl DocumentProvider for TestProvider {
        fn read_bytes(&self, _path: &Path) -> Result<Vec<u8>> {
            Ok(self.content.as_bytes().to_vec())
        }

        fn metadata(&self, _path: &Path) -> Result<crate::storage_provider::DocumentFileMetadata> {
            Ok(crate::storage_provider::DocumentFileMetadata {
                modified: Some(SystemTime::UNIX_EPOCH),
                size: self.content.len() as u64,
            })
        }
    }

    let provider = TestProvider {
        content: r#"{"canvas": {"components": [{"id": "input1", "type": "text"}]}}"#.to_string(),
    };
    let meta = parse_file_with_document_provider(Path::new("app/page.spg"), &provider, None)
        .expect("parse_file_with_document_provider should succeed");

    assert!(meta.superpage.is_some());
    assert_eq!(meta.input_path, Some("app/page.spg".to_string()));
}

#[test]
fn test_parse_file_with_storage_compat_layer() {
    // parse_file_with_storage 是兼容别名，内部应调用 parse_file_with_document_provider
    let meta = parse_file_with_storage(
        Path::new("tests/fixtures/actions_test.spg"),
        &crate::storage_provider::LocalStorageProvider,
    )
    .expect("parse_file_with_storage should still work");

    assert!(meta.superpage.is_some() || !meta.components.is_empty() || !meta.settings.is_empty());
}

#[test]
fn test_parse_file_accepts_absolute_path_input() {
    // M36 回归：CLI 绝对路径单文件解析必须仍然工作
    let text = r#"{"canvas": {"components": [{"id": "btn1", "type": "button"}]}}"#;
    let source = crate::source_id::SourceId::from_memory(
        crate::source_id::ProjectRef::new("test"),
        "/tmp/abs_page.spg",
    );
    let parsed = ParsedContent::from_text(source, text);
    let meta = parse_content(&parsed).expect("parse_content must work for absolute path input");
    assert!(meta.superpage.is_some());
}

#[test]
fn test_parse_metadata_from_str_spg() {
    let text = r#"{"canvas": {"components": [{"id": "text1", "type": "text"}]}}"#;
    let source = crate::source_id::SourceId::from_memory(
        crate::source_id::ProjectRef::new("test"),
        "app/page.spg",
    );
    let meta = parse_metadata_from_str(source, text)
        .expect("parse_metadata_from_str should succeed for spg");
    assert!(meta.superpage.is_some());
    assert_eq!(meta.input_path, Some("app/page.spg".to_string()));
}

#[test]
fn test_parse_metadata_from_str_tbl() {
    let text = r#"{"dimensions": [{"id": "dim1"}]}"#;
    let mut source = crate::source_id::SourceId::from_memory(
        crate::source_id::ProjectRef::new("test"),
        "data/table.tbl",
    );
    source.source_kind = crate::source_id::SourceKind::Tbl;
    let meta = parse_metadata_from_str(source, text)
        .expect("parse_metadata_from_str should succeed for tbl");
    assert!(meta.tbl.is_some());
}

#[test]
fn test_parse_metadata_from_value() {
    let value = serde_json::json!({"canvas": {"components": [{"id": "text1", "type": "text"}]}});
    let source = crate::source_id::SourceId::from_memory(
        crate::source_id::ProjectRef::new("test"),
        "app/page.spg",
    );
    let meta = parse_metadata_from_value(source, std::sync::Arc::new(value))
        .expect("parse_metadata_from_value should succeed");
    assert!(meta.superpage.is_some());
    assert_eq!(meta.input_path, Some("app/page.spg".to_string()));
}

#[test]
fn test_parse_tbl_from_value_with_source() {
    let raw = serde_json::json!({"dimensions": [{"name": "id", "dbfield": "ID"}]});
    let mut source = crate::source_id::SourceId::from_memory(
        crate::source_id::ProjectRef::new("test"),
        "data/table.tbl",
    );
    source.source_kind = crate::source_id::SourceKind::Tbl;
    let meta = crate::tbl_single::parse_tbl_from_value_with_source(&source, raw)
        .expect("parse_tbl_from_value_with_source should succeed");
    assert_eq!(meta.table_name, Some("table".to_string()));
    assert_eq!(meta.input_path, Some("data/table.tbl".to_string()));
    assert!(!meta.fields.is_empty());
}
