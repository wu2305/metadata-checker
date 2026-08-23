use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// 返回 benchmark 模拟外部进程占用 graphdb 时使用的锁文件路径。
pub fn graph_lock_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("graphdb.lock")
}

/// benchmark 外部锁守卫，释放时同步删除锁文件。
#[must_use]
pub struct ExternalGraphLock {
    lock_file: Option<File>,
}

impl Drop for ExternalGraphLock {
    fn drop(&mut self) {
        if let Some(lock_file) = self.lock_file.take() {
            let _ = lock_file.unlock();
        }
    }
}

/// 获取用于模拟外部 graphdb 占用的锁。
pub fn acquire_external_graph_lock(db_path: &Path) -> Result<ExternalGraphLock> {
    let lock_path = graph_lock_path(db_path);
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .with_context(|| format!("acquire external graph lock {}", lock_path.display()))?;
    lock_file
        .try_lock_exclusive()
        .with_context(|| format!("lock external graph lock {}", lock_path.display()))?;

    Ok(ExternalGraphLock {
        lock_file: Some(lock_file),
    })
}
