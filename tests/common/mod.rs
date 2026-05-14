use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// 构建 fixture graphdb 到唯一临时目录
///
/// 返回 (temp_dir, db_path)。调用方在测试结束后应清理 temp_dir。
pub fn build_fixture_graphdb() -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
    let unique = format!("metadata-checker-fixture-{}-{}", nanos, seq);
    let temp_dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)
        .expect("create temp dir must succeed");

    let src = std::path::Path::new("tests/fixtures/test_project");
    copy_dir_all(src, &temp_dir)
        .expect("copy fixture dir must succeed");

    let db_path = temp_dir.join(".metadata-checker.graphdb");
    metadata_checker::scanner::scan_project(&temp_dir, &db_path,
    ).expect("scan_project must succeed");
    (temp_dir, db_path)
}

fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry_result in std::fs::read_dir(src)? {
        let entry = entry_result
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("read_dir entry failed: {}", e)))?;
        let ty = entry.file_type()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("file_type failed for {:?}: {}", entry.path(), e)))?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("copy_dir_all {:?} -> {:?}: {}", entry.path(), dst.as_ref().join(entry.file_name()), e)))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("copy {:?} -> {:?}: {}", entry.path(), dst.as_ref().join(entry.file_name()), e)))?;
        }
    }
    Ok(())
}
