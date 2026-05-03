use serde_json::Value;
use std::fs;
use std::path::Path;

fn load_manifest() -> Value {
    let path = Path::new("tests/fixtures/corpus/manifest.json");
    let content = fs::read_to_string(path).expect("manifest.json should be readable");
    serde_json::from_str(&content).expect("manifest.json should be valid JSON")
}

#[test]
fn test_corpus_manifest_file_exists() {
    let path = Path::new("tests/fixtures/corpus/manifest.json");
    assert!(path.exists(), "manifest.json should exist");
}

#[test]
fn test_corpus_manifest_schema_basic_fields() {
    let manifest = load_manifest();

    let sample_count = manifest
        .get("sample_count")
        .and_then(Value::as_u64)
        .expect("sample_count should exist and be u64");
    let samples = manifest
        .get("samples")
        .and_then(Value::as_array)
        .expect("samples should exist and be an array");
    assert_eq!(
        sample_count as usize,
        samples.len(),
        "sample_count should match samples length"
    );

    assert!(!samples.is_empty(), "samples should not be empty");

    for sample in samples {
        let stable_id = sample
            .get("stable_id")
            .and_then(Value::as_str)
            .expect("stable_id should be string");
        assert!(!stable_id.is_empty(), "stable_id should not be empty");

        let source_kind = sample
            .get("source_kind")
            .and_then(Value::as_str)
            .expect("source_kind should be string");
        assert!(
            source_kind == "real_project" || source_kind == "fixture",
            "source_kind should be real_project or fixture"
        );

        let path = sample
            .get("path")
            .and_then(Value::as_str)
            .expect("path should be string");
        assert!(!path.is_empty(), "path should not be empty");

        let file_type = sample
            .get("file_type")
            .and_then(Value::as_str)
            .expect("file_type should be string");
        assert!(
            file_type == "spg" || file_type == "tbl",
            "file_type should be spg or tbl"
        );

        sample
            .get("size_bytes")
            .and_then(Value::as_u64)
            .expect("size_bytes should be u64");

        let coverage_tags = sample
            .get("coverage_tags")
            .and_then(Value::as_array)
            .expect("coverage_tags should be array");
        for tag_obj in coverage_tags {
            let tag = tag_obj
                .get("tag")
                .and_then(Value::as_str)
                .expect("coverage_tags[].tag should be string");
            assert!(!tag.is_empty(), "coverage tag should not be empty");
            let detection_method = tag_obj
                .get("detection_method")
                .and_then(Value::as_str)
                .expect("coverage_tags[].detection_method should be string");
            assert!(
                !detection_method.is_empty(),
                "detection_method should not be empty"
            );
        }
    }
}

#[test]
fn test_corpus_manifest_source_path_constraints() {
    let manifest = load_manifest();
    let samples = manifest
        .get("samples")
        .and_then(Value::as_array)
        .expect("samples should exist and be an array");

    let allowed_real_root =
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi/";
    for sample in samples {
        let source_kind = sample
            .get("source_kind")
            .and_then(Value::as_str)
            .expect("source_kind should be string");
        let path = sample
            .get("path")
            .and_then(Value::as_str)
            .expect("path should be string");

        assert!(
            !path.contains("/Users/wuhaocheng/Downloads/bi"),
            "path should not include forbidden source /Users/wuhaocheng/Downloads/bi: {path}"
        );

        if source_kind == "real_project" {
            assert!(
                path.starts_with(allowed_real_root),
                "real_project path should start with allowed root: {path}"
            );
        } else {
            assert!(
                path.starts_with("tests/fixtures/"),
                "fixture path should stay inside tests/fixtures: {path}"
            );
        }
    }
}

#[test]
fn test_corpus_manifest_key_tag_coverage() {
    let manifest = load_manifest();
    let coverage_matrix = manifest
        .get("coverage_matrix")
        .and_then(Value::as_object)
        .expect("coverage_matrix should be an object");

    for tag in [
        "showDialog",
        "DataFlow",
        "waitPrev",
        "visibility",
        "refreshData",
        "conditionExp",
    ] {
        let entry = coverage_matrix
            .get(tag)
            .expect("key tag should exist in coverage_matrix");
        let sample_count = entry
            .get("sample_count")
            .and_then(Value::as_u64)
            .expect("sample_count should be u64");
        assert!(
            sample_count > 0,
            "key tag should have at least one sample: {tag}"
        );
    }
}
