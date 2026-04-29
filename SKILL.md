---
name: metadata-checker
description: |
  Use the metadata-checker CLI tool to parse and analyze SuperPage (.spg) and Table (.tbl) metadata files from a low-code platform.
  This skill guides you on when and how to invoke the tool to extract component trees, expressions, dependencies,
  value source traces, and calculation priority analysis from page metadata JSON files.

  Use this skill when:
  1. You need to parse or analyze a .spg file (low-code platform SuperPage metadata).
  2. You need to trace the source of a component's value (e.g., which model/param/component it comes from).
  3. You need to build a dependency graph of components and detect cycles.
  4. You need to analyze calculation priority rules (defaultValue vs exp vs calcCondition).
  5. You need to extract structured metadata from a low-code platform page for downstream processing.
---

# metadata-checker Skill

## Overview

The `metadata-checker` is a Rust CLI tool that parses `.spg` files (SuperPage metadata JSON) and `.tbl` files (Table/DataFlow metadata) from a low-code platform.

## Task Decision Tree

When working with metadata-checker, follow this decision tree to choose the right command:

### Q1: Do you have a single `.spg` file to analyze?

**Yes** → Use `metadata-checker <FILE.spg>` (default JSON output)

- Need a compact overview? → Default output (no flags)
- Need full details (components, expressions, dependency_order)? → Add `--detail`
- Need priority analysis (defaultValue vs exp vs calcCondition)? → Add `--priority`
- Need to query a specific component? → Add `--query <COMPONENT_ID>`
- Need human-readable text? → Add `--human` (expert exploration mode)

### Q2: Do you need to understand what a specific ID does?

**Yes** → Use `metadata-checker --explain <ID>`

- Component in a single file → `--explain input1` (with `--project-dir` for cross-file context)
- Model/field/page/dataflow in project graph → `--explain model:physical_x`

Output includes: `what_is_it`, `reads`, `writes`, `triggered_by`, `affects`, `lineage`, `evidence`

### Q3: Do you need the surrounding context of an ID?

**Yes** → Use `metadata-checker --context <ID> --depth <N> --budget <compact|normal|full>`

- Default depth is 1, default budget is `normal`.
- Use `--budget compact` to avoid large raw JSON.

### Q4: Do you need project-level analysis?

**Yes** → You need `--project-dir <DIR>`

First, build the graph database:
```bash
metadata-checker --project-dir /path/to/project --build-graph
```

Then query:
- Model read/write relationships → `--query-model <MODEL>`
- Page dependencies → `--query-page <PAGE>`
- Cross-file relations between two pages → `--query-cross <A> <B>`
- DataFlow subgraph → `--query-dataflow <MODEL>`
- Page-level logic summary → `--query-page-logic <PAGE>`

### Q5: Do you need page-level logic summary?

**Yes** → Use `metadata-checker --project-dir /path/to/project --query-page-logic <page>`

Output includes: `page_inputs`, `data_sources`, `write_targets`, `entrypoints`, `action_flows`, `visibility_rules`, `navigation`, `risk_diagnostics`

## Machine JSON Output Schema

All machine outputs (`--non-human`, default) follow a unified top-level structure:

```json
{
  "schema_version": "1.0",
  "kind": "SuperPage | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic",
  "query_target": "...",
  "summary": { /* Low-noise summary, AI should read this first */ },
  "details": { /* Detailed data, only when needed */ },
  "evidence": [ /* Evidence chain for every conclusion */ ],
  "diagnostics": [ /* Warnings, errors, unresolved refs */ ],
  "next_queries": [ /* Suggested follow-up CLI commands */ ]
}
```

**AI Usage Rule**: Always read `summary` first. Only read `details` or `evidence` when you need to verify a specific claim. Never read raw JSON by default. If `diagnostics` contains entries, you must give a conservative answer.

## Important Constraints

1. `--project-dir` is **required** for all project-level queries (`--query-model`, `--query-page`, `--query-cross`, `--query-dataflow`, `--explain`, `--context`, `--query-page-logic`). Without it, the tool exits with an error.
2. `--human` and `--interactive` both enter REPL mode for expert exploration. Do not use them for automated/machine consumption.
3. Component IDs in expressions use the full path format: `comp:app/page.spg|component_id`.
4. Page node IDs use normalized relative paths: `page:app/page.spg`.
5. Legacy compatibility fields exist but should not be used as primary semantics:
   - `upstream_dependencies` → use `produced_by` / `consumed_by_dataflows` / `dataflow_inputs`
   - `downstream_outputs` → use `dataflow_outputs` / `produced_by`

## Examples

```bash
# Parse single file, compact JSON output (default)
metadata-checker page.spg

# Parse with full details
metadata-checker page.spg --detail

# Query specific component
metadata-checker page.spg --query input3

# Explain a component
metadata-checker page.spg --explain input1

# Build graph for project
metadata-checker --project-dir /path/to/project --build-graph

# Query model (JSON output)
metadata-checker --project-dir /path/to/project --query-model model1

# Query model (human output)
metadata-checker --project-dir /path/to/project --query-model model1 --human

# Get context around a button
metadata-checker --project-dir /path/to/project --context button1 --depth 2 --budget compact

# Page logic summary
metadata-checker --project-dir /path/to/project --query-page-logic "page:app/合同管理/销售合同.spg"
```
