use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use metadata_checker::persistence::GraphPersistenceProvider;

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
    std::fs::create_dir_all(&temp_dir).expect("create temp dir must succeed");

    let src = std::path::Path::new("tests/fixtures/test_project");
    copy_dir_all(src, &temp_dir).expect("copy fixture dir must succeed");

    let db_path = temp_dir.join(".metadata-checker.graphdb");
    metadata_checker::scanner::scan_project(&temp_dir, &db_path)
        .expect("scan_project must succeed");
    // redb 的 pending freed pages 在 Database::open + drop 时才落盘，会改变
    // 文件大小。扫描后用 RedbPersistenceProvider 的读路径（与下游测试探针
    // 同源）settle 一次，让量到的 before_len 是稳定基线（M58.3 复核返修：
    // scanner 诊断并入 persist 事务后，构建末尾少了独立 save 事务，pending
    // 状态留给了下一个打开者；GraphDB::open 的写事务路径不会触发该落盘）。
    {
        let provider = metadata_checker::persistence::redb::RedbPersistenceProvider::new(&db_path)
            .expect("settle provider must init");
        let _ = provider.load_graph_meta("project:settle", "graph:settle");
    }
    (temp_dir, db_path)
}

/// 判断是否应跳过的 fixture 运行产物
fn should_skip_fixture_artifact(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    name == ".metadata-checker.graphdb"
        || name == ".metadata-checker.graphdb.lock"
        || name.ends_with(".graphdb")
        || name.ends_with(".graphdb.lock")
}

fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry_result in std::fs::read_dir(src)? {
        let entry = entry_result.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("read_dir entry failed: {}", e),
            )
        })?;
        if should_skip_fixture_artifact(&entry.path()) {
            continue;
        }
        let ty = entry.file_type().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("file_type failed for {:?}: {}", entry.path(), e),
            )
        })?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name())).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "copy_dir_all {:?} -> {:?}: {}",
                        entry.path(),
                        dst.as_ref().join(entry.file_name()),
                        e
                    ),
                )
            })?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name())).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "copy {:?} -> {:?}: {}",
                        entry.path(),
                        dst.as_ref().join(entry.file_name()),
                        e
                    ),
                )
            })?;
        }
    }
    Ok(())
}
