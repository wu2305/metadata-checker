#[path = "../benches/common/external_graph_lock.rs"]
mod external_graph_lock;

use external_graph_lock::{acquire_external_graph_lock, graph_lock_path};

#[test]
fn test_external_graph_lock_drop_releases_lock_for_next_iteration() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "metadata-checker-bench-lock-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    let db_path = root.join("metadata-checker.graphdb");
    let lock_path = graph_lock_path(&db_path);

    {
        let _lock = acquire_external_graph_lock(&db_path)?;
        assert!(lock_path.exists());
        assert!(acquire_external_graph_lock(&db_path).is_err());
    }

    assert!(lock_path.exists());
    {
        let _lock = acquire_external_graph_lock(&db_path)?;
        assert!(lock_path.exists());
    }
    assert!(lock_path.exists());

    std::fs::remove_file(&lock_path)?;
    std::fs::remove_dir_all(&root)?;
    Ok(())
}
