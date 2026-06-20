//! 一次性扫描真实项目，生成 browser offscreen bench 的 fixture 与 manifest。
//!
//! 只负责样本选择与文件复制；benchmark 运行时只消费已入仓产物。

use anyhow::{Context, Result, bail};
use metadata_checker::superpage::{self, SuperPageMetadata};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const SAMPLE_IDS: [&str; 5] = [
    "typical_p75_page",
    "large_raw_page",
    "high_component_page",
    "high_reference_page",
    "worst_combined_page",
];

const COMPONENT_KINDS: [&str; 3] = [
    "high_reference_component",
    "container_component",
    "leaf_component",
];

/// 单页扫描统计
#[derive(Debug, Clone)]
struct PageStats {
    source_path: String,
    raw_bytes: usize,
    component_count: usize,
    reference_count: usize,
    meta: SuperPageMetadata,
}

/// manifest 中的组件候选摘要
#[derive(Debug, Clone, Serialize)]
struct ComponentCandidate {
    id: String,
    component_type: String,
    reference_count: usize,
    child_count: usize,
}

/// manifest 中的页面样本
#[derive(Debug, Clone, Serialize)]
struct PageSample {
    id: String,
    source_path: String,
    component_count: usize,
    reference_count: usize,
    raw_bytes: usize,
    components: HashMap<String, ComponentCandidate>,
}

/// manifest 根结构
#[derive(Debug, Serialize)]
struct FixtureManifest {
    schema_version: u32,
    project_name: String,
    fixture_root: String,
    samples: Vec<PageSample>,
}

fn is_container_type(component_type: &str) -> bool {
    let lowered = component_type.to_ascii_lowercase();
    ["panel", "layout", "container", "tab", "panelbook", "steps"]
        .iter()
        .any(|token| lowered.contains(token))
}

fn count_component_refs(meta: &SuperPageMetadata) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for expr in &meta.expressions {
        *counts.entry(expr.component_id.clone()).or_insert(0) += expr.refs.len();
    }
    counts
}

fn child_counts(meta: &SuperPageMetadata) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for component in &meta.components {
        if let Some(parent_id) = &component.parent_id {
            *counts.entry(parent_id.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn pick_component_candidates(
    meta: &SuperPageMetadata,
) -> Result<HashMap<String, ComponentCandidate>> {
    if meta.components.is_empty() {
        bail!("page has no components");
    }
    let ref_counts = count_component_refs(meta);
    let children = child_counts(meta);
    let mut picked = HashMap::new();

    let high_ref = meta
        .components
        .iter()
        .max_by_key(|component| ref_counts.get(&component.id).copied().unwrap_or(0))
        .context("missing high reference component")?;
    picked.insert(
        COMPONENT_KINDS[0].to_string(),
        ComponentCandidate {
            id: high_ref.id.clone(),
            component_type: high_ref.component_type.clone(),
            reference_count: ref_counts.get(&high_ref.id).copied().unwrap_or(0),
            child_count: children.get(&high_ref.id).copied().unwrap_or(0),
        },
    );

    let container = meta
        .components
        .iter()
        .filter(|component| {
            children.get(&component.id).copied().unwrap_or(0) > 0
                || is_container_type(&component.component_type)
        })
        .max_by_key(|component| children.get(&component.id).copied().unwrap_or(0))
        .or_else(|| meta.components.first());
    let container = container.context("missing container component")?;
    picked.insert(
        COMPONENT_KINDS[1].to_string(),
        ComponentCandidate {
            id: container.id.clone(),
            component_type: container.component_type.clone(),
            reference_count: ref_counts.get(&container.id).copied().unwrap_or(0),
            child_count: children.get(&container.id).copied().unwrap_or(0),
        },
    );

    let leaf = meta
        .components
        .iter()
        .filter(|component| children.get(&component.id).copied().unwrap_or(0) == 0)
        .min_by_key(|component| ref_counts.get(&component.id).copied().unwrap_or(0))
        .or_else(|| meta.components.last());
    let leaf = leaf.context("missing leaf component")?;
    picked.insert(
        COMPONENT_KINDS[2].to_string(),
        ComponentCandidate {
            id: leaf.id.clone(),
            component_type: leaf.component_type.clone(),
            reference_count: ref_counts.get(&leaf.id).copied().unwrap_or(0),
            child_count: 0,
        },
    );

    Ok(picked)
}

fn collect_spg_files(dir: &Path, project_dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        fs::read_dir(dir).with_context(|| format!("failed to read dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_spg_files(&path, project_dir, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("spg") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan_project(project_dir: &Path) -> Result<Vec<PageStats>> {
    let mut paths = Vec::new();
    collect_spg_files(project_dir, project_dir, &mut paths)?;
    let mut pages = Vec::new();
    for path in paths {
        let rel = path
            .strip_prefix(project_dir)
            .with_context(|| format!("failed to relativize {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let raw_bytes = fs::metadata(&path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .len() as usize;
        let meta = superpage::parse_superpage(&path)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let reference_count = meta.expressions.iter().map(|expr| expr.refs.len()).sum();
        pages.push(PageStats {
            source_path: rel,
            raw_bytes,
            component_count: meta.components.len(),
            reference_count,
            meta,
        });
    }
    if pages.is_empty() {
        bail!("no .spg files found under {}", project_dir.display());
    }
    Ok(pages)
}

fn percentile_index(len: usize, percentile: f64) -> usize {
    if len == 0 {
        return 0;
    }
    let index = ((len as f64 - 1.0) * percentile).round() as usize;
    index.min(len - 1)
}

fn normalized_score(
    page: &PageStats,
    max_bytes: usize,
    max_components: usize,
    max_refs: usize,
) -> f64 {
    let bytes = if max_bytes == 0 {
        0.0
    } else {
        page.raw_bytes as f64 / max_bytes as f64
    };
    let components = if max_components == 0 {
        0.0
    } else {
        page.component_count as f64 / max_components as f64
    };
    let refs = if max_refs == 0 {
        0.0
    } else {
        page.reference_count as f64 / max_refs as f64
    };
    bytes + components + refs
}

fn pick_samples(pages: &[PageStats]) -> Result<Vec<usize>> {
    let max_bytes = pages.iter().map(|page| page.raw_bytes).max().unwrap_or(0);
    let max_components = pages
        .iter()
        .map(|page| page.component_count)
        .max()
        .unwrap_or(0);
    let max_refs = pages
        .iter()
        .map(|page| page.reference_count)
        .max()
        .unwrap_or(0);

    let mut by_bytes: Vec<usize> = (0..pages.len()).collect();
    by_bytes.sort_by_key(|index| pages[*index].raw_bytes);
    let typical_index = by_bytes[percentile_index(by_bytes.len(), 0.75)];

    let pick_max_index = |selector: fn(&PageStats) -> usize| {
        pages
            .iter()
            .enumerate()
            .max_by_key(|(_index, page)| selector(page))
            .map(|(index, _)| index)
    };

    let large_raw_index =
        pick_max_index(|page| page.raw_bytes).context("missing large raw page")?;
    let high_component_index =
        pick_max_index(|page| page.component_count).context("missing high component page")?;
    let high_reference_index =
        pick_max_index(|page| page.reference_count).context("missing high reference page")?;

    let mut selected = Vec::new();
    let mut seen_paths = HashSet::new();

    let push_unique =
        |index: usize, selected: &mut Vec<usize>, seen_paths: &mut HashSet<String>| {
            let source_path = pages[index].source_path.clone();
            if seen_paths.insert(source_path) {
                selected.push(index);
                true
            } else {
                false
            }
        };

    push_unique(typical_index, &mut selected, &mut seen_paths);
    push_unique(large_raw_index, &mut selected, &mut seen_paths);
    push_unique(high_component_index, &mut selected, &mut seen_paths);
    push_unique(high_reference_index, &mut selected, &mut seen_paths);

    let worst_combined_index = pages
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected.contains(index))
        .max_by(|(_left_index, left), (_right_index, right)| {
            normalized_score(left, max_bytes, max_components, max_refs)
                .partial_cmp(&normalized_score(
                    right,
                    max_bytes,
                    max_components,
                    max_refs,
                ))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .or_else(|| {
            pages
                .iter()
                .enumerate()
                .max_by(|(_left_index, left), (_right_index, right)| {
                    normalized_score(left, max_bytes, max_components, max_refs)
                        .partial_cmp(&normalized_score(
                            right,
                            max_bytes,
                            max_components,
                            max_refs,
                        ))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(index, _)| index)
        })
        .context("missing worst combined page")?;
    push_unique(worst_combined_index, &mut selected, &mut seen_paths);

    if selected.len() < SAMPLE_IDS.len() {
        let mut ranked: Vec<(usize, f64)> = pages
            .iter()
            .enumerate()
            .filter(|(index, _)| !selected.contains(index))
            .map(|(index, page)| {
                (
                    index,
                    normalized_score(page, max_bytes, max_components, max_refs),
                )
            })
            .collect();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (index, _score) in ranked {
            if selected.len() >= SAMPLE_IDS.len() {
                break;
            }
            push_unique(index, &mut selected, &mut seen_paths);
        }
    }

    if selected.len() < SAMPLE_IDS.len() {
        bail!(
            "need at least {} distinct pages, found {}",
            SAMPLE_IDS.len(),
            selected.len()
        );
    }
    Ok(selected.into_iter().take(SAMPLE_IDS.len()).collect())
}

fn copy_fixture(project_dir: &Path, fixture_root: &Path, source_path: &str) -> Result<()> {
    let from = project_dir.join(source_path);
    let to = fixture_root.join(source_path);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&from, &to)
        .with_context(|| format!("failed to copy {} -> {}", from.display(), to.display()))?;
    Ok(())
}

fn parse_args() -> Result<(PathBuf, PathBuf, String)> {
    let mut project_dir = None;
    let mut output_dir = None;
    let mut project_name = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project-dir" => {
                project_dir = Some(PathBuf::from(
                    args.next().context("missing value for --project-dir")?,
                ));
            }
            "--output-dir" => {
                output_dir = Some(PathBuf::from(
                    args.next().context("missing value for --output-dir")?,
                ));
            }
            "--project-name" => {
                project_name = Some(args.next().context("missing value for --project-name")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: generate_browser_offscreen_fixtures --project-dir DIR --output-dir DIR [--project-name NAME]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    let project_dir = project_dir.context("missing --project-dir")?;
    let output_dir = output_dir.context("missing --output-dir")?;
    let project_name = project_name.unwrap_or_else(|| {
        project_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string()
    });
    Ok((project_dir, output_dir, project_name))
}

fn main() -> Result<()> {
    let (project_dir, output_dir, project_name) = parse_args()?;
    if !project_dir.is_dir() {
        bail!("project dir not found: {}", project_dir.display());
    }

    let pages = scan_project(&project_dir)?;
    let selected_indices = pick_samples(&pages)?;
    let fixture_root = output_dir.join("project");
    if fixture_root.exists() {
        fs::remove_dir_all(&fixture_root)
            .with_context(|| format!("failed to clean {}", fixture_root.display()))?;
    }
    fs::create_dir_all(&fixture_root)
        .with_context(|| format!("failed to create {}", fixture_root.display()))?;

    let mut samples = Vec::new();
    for (index, page_index) in selected_indices.iter().enumerate() {
        let page = &pages[*page_index];
        copy_fixture(&project_dir, &fixture_root, &page.source_path)?;
        let components = pick_component_candidates(&page.meta)?;
        samples.push(PageSample {
            id: SAMPLE_IDS[index].to_string(),
            source_path: page.source_path.clone(),
            component_count: page.component_count,
            reference_count: page.reference_count,
            raw_bytes: page.raw_bytes,
            components,
        });
    }

    let manifest = FixtureManifest {
        schema_version: 1,
        project_name,
        fixture_root: "project".to_string(),
        samples,
    };
    let manifest_path = output_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).context("failed to serialize manifest")?,
    )
    .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "manifest": manifest_path,
            "fixture_root": fixture_root,
            "sample_count": manifest.samples.len(),
        })
    );
    Ok(())
}
