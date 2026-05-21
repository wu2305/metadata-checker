use std::path::Path;

/// Resolve a reference path from referenceResources to an absolute path.
/// Handles relative paths (../, ./) and $TAPP: prefix.
pub fn resolve_reference_path(
    rel_path: &str,
    ref_idx: usize,
    reference_resources: &[String],
) -> Option<String> {
    let ref_path = reference_resources.get(ref_idx)?;
    if ref_path.starts_with("$TAPP:") {
        // $TAPP:/path/to/page.spg → resolve relative to project root
        let path = ref_path.strip_prefix("$TAPP:").unwrap_or(ref_path);
        Some(path.trim_start_matches('/').to_string())
    } else {
        // Relative path: resolve based on current .spg directory
        let current_dir = Path::new(rel_path).parent()?;
        let resolved = current_dir.join(ref_path);
        Some(resolved.to_string_lossy().to_string().replace('\\', "/"))
    }
}
