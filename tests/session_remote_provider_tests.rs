#![cfg(feature = "cli-local")]

use metadata_checker::remote_metadata::{RemoteFileContent, RemoteFileRef};
use metadata_checker::session::remote_provider::{
    InMemoryRemoteSessionProvider, RemoteMetafileEntry, RemoteProjectInfo, RemoteSessionProvider,
    TestRemoteSessionProvider,
};

fn build_file_ref(project_ref: &str, source_path: &str, file_id: Option<&str>) -> RemoteFileRef {
    RemoteFileRef {
        remote_ref: None,
        project_ref: project_ref.to_string(),
        source_path: source_path.to_string(),
        file_id: file_id.map(str::to_string),
        revision: None,
    }
}

fn seed_provider() -> InMemoryRemoteSessionProvider {
    let mut provider = InMemoryRemoteSessionProvider::new();
    provider
        .register_project(RemoteProjectInfo {
            project_ref: "project-1".to_string(),
            project_name: "销售项目".to_string(),
            source_origin: "remote".to_string(),
        })
        .unwrap();

    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: "project-1".to_string(),
                source_path: "app/pageA.spg".to_string(),
                file_id: Some("file-a".to_string()),
                revision: Some("1".to_string()),
                etag: Some("etag-a".to_string()),
                mtime: Some(100),
                size: Some(10),
                deleted: false,
            },
            RemoteFileContent {
                source_path: "app/pageA.spg".to_string(),
                file_id: Some("file-a".to_string()),
                revision: Some("1".to_string()),
                content_type: metadata_checker::remote_metadata::MetadataContentType::SuperPage,
                raw_text: r#"{"canvas":{"components":[]}}"#.to_string(),
            },
        )
        .unwrap();
    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: "project-1".to_string(),
                source_path: "app/pageB.spg".to_string(),
                file_id: Some("file-b".to_string()),
                revision: Some("2".to_string()),
                etag: Some("etag-b".to_string()),
                mtime: Some(120),
                size: Some(11),
                deleted: false,
            },
            RemoteFileContent {
                source_path: "app/pageB.spg".to_string(),
                file_id: Some("file-b".to_string()),
                revision: Some("2".to_string()),
                content_type: metadata_checker::remote_metadata::MetadataContentType::SuperPage,
                raw_text: r#"{"canvas":{"components":[1]}}"#.to_string(),
            },
        )
        .unwrap();
    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: "project-1".to_string(),
                source_path: "app/pageC.tbl".to_string(),
                file_id: Some("file-c".to_string()),
                revision: Some("3".to_string()),
                etag: Some("etag-c".to_string()),
                mtime: Some(140),
                size: Some(12),
                deleted: true,
            },
            RemoteFileContent {
                source_path: "app/pageC.tbl".to_string(),
                file_id: Some("file-c".to_string()),
                revision: Some("3".to_string()),
                content_type: metadata_checker::remote_metadata::MetadataContentType::Table,
                raw_text: r#"{"table":{"fields":[]}}"#.to_string(),
            },
        )
        .unwrap();

    provider
}

#[test]
fn list_projects_and_metafiles() {
    let provider = seed_provider();
    let projects = provider.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_ref, "project-1");
    assert_eq!(projects[0].project_name, "销售项目");

    let files = provider.list_metafiles("project-1").unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].source_path, "app/pageA.spg");
    assert_eq!(files[1].source_path, "app/pageB.spg");
    assert_eq!(files[2].source_path, "app/pageC.tbl");
}

#[test]
fn fetch_metafile_info_and_content() {
    let provider = seed_provider();
    let file_ref = build_file_ref("project-1", "app/pageA.spg", Some("file-a"));
    let info = provider.fetch_metafile_info(&file_ref).unwrap();
    assert_eq!(info.file_id, Some("file-a".to_string()));
    assert_eq!(info.revision, Some("1".to_string()));

    let content = provider.fetch_metafile_content(&file_ref).unwrap();
    assert_eq!(content.file_id, Some("file-a".to_string()));
    assert_eq!(content.revision, Some("1".to_string()));
}

#[test]
fn fetch_changed_since_filters_revision() {
    let provider = seed_provider();
    let changed = provider.fetch_changed_since("project-1", "1").unwrap();
    assert_eq!(changed.project_ref, "project-1");
    assert_eq!(changed.project_name, "销售项目");
    assert_eq!(changed.since_revision, "1");
    assert_eq!(changed.changed_files.len(), 2);
    assert_eq!(changed.changed_files[0].revision, Some("2".to_string()));
    assert_eq!(changed.changed_files[1].source_path, "app/pageC.tbl");
}

#[test]
fn fetch_missing_file_returns_error() {
    let provider = seed_provider();
    let miss =
        provider.fetch_metafile_content(&build_file_ref("project-1", "app/missing.spg", None));
    let err = miss.unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn debug_does_not_leak_secrets() {
    let mut provider = TestRemoteSessionProvider::new();
    provider
        .register_project(RemoteProjectInfo {
            project_ref: "project-2".to_string(),
            project_name: "测试项目".to_string(),
            source_origin: "remote".to_string(),
        })
        .unwrap();
    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: "project-2".to_string(),
                source_path: "app/secret.spg".to_string(),
                file_id: Some("file-secret".to_string()),
                revision: Some("4".to_string()),
                etag: Some("etag-secret".to_string()),
                mtime: Some(201),
                size: Some(32),
                deleted: false,
            },
            RemoteFileContent {
                source_path: "app/secret.spg".to_string(),
                file_id: Some("file-secret".to_string()),
                revision: Some("4".to_string()),
                content_type: metadata_checker::remote_metadata::MetadataContentType::SuperPage,
                raw_text: "token=secret-token cookie=session-cookie password=secret-passwd"
                    .to_string(),
            },
        )
        .unwrap();

    let debug_text = format!("{provider:?}");
    assert!(!debug_text.contains("secret-token"));
    assert!(!debug_text.contains("session-cookie"));
    assert!(!debug_text.contains("secret-passwd"));
}
