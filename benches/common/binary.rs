use std::path::PathBuf;

/// 解析 release 二进制路径，优先 `release-fast`（与 Makefile 一致）。
pub fn resolve_release_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_metadata-checker") {
        let path = PathBuf::from(path);
        if path.exists() {
            return path;
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for profile in ["release-fast", "release"] {
        let candidate = manifest
            .join("target")
            .join(profile)
            .join("metadata-checker");
        if candidate.exists() {
            return candidate;
        }
    }
    manifest.join("target/release-fast/metadata-checker")
}
