---
name: metadata-checker
description: |
  Use the metadata-checker CLI tool to parse and analyze SuperPage (.spg) metadata files from a low-code platform.
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

The `metadata-checker` is a Rust CLI tool that parses `.spg` files (SuperPage metadata JSON) from a low-code platform.
It extracts:
- Component trees (with parent-child relationships)
- Expressions from 30+ fields (value, exp, defaultValue, visible, itemFilter, validExp, calcCondition, calcExp, etc.)
- Reference types within expressions (model fields, parameters, component values/properties, user properties, system variables)
- Dependency graphs and topological sort order
- Cycle detection
- Calculation priority analysis (`--priority` flag)

## Binary Location

The binary is built in the workspace at:
```
/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker
```

## Usage

### Basic Command

```bash
metadata-checker <FILE.spg> [OPTIONS]
```

### Options

| Flag | Description |
|------|-------------|
| `--human` | Enter interactive REPL for expert exploration |
| `--interactive` | Alias for `--human`, also enters REPL |
| `--non-human` | Machine-friendly compact JSON output (default) |
| `--priority` | Additionally merge priority analysis into main JSON output |
| `--query <ID>` | Query detailed info for a specific component ID |
| `--project-dir <DIR>` | Project directory for cross-file graph analysis |
| `--build-graph` | Scan project directory and build/update graph database |
| `--query-model <MODEL>` | Query model read/write relationships (requires `--project-dir`) |
| `--query-page <PAGE>` | Query page dependencies (requires `--project-dir`) |
| `--query-cross <A> <B>` | Query cross-file relations between two pages (requires `--project-dir`) |
| `--query-dataflow <MODEL>` | Expand DataFlow subgraph (requires `--project-dir`) |

### Output Modes

**`--human` mode:** Enters interactive REPL for expert exploration. You can input component IDs to query their details interactively.

**`--interactive` mode:** Enter REPL where you can input component IDs to get detailed query results interactively.

**`--non-human` mode (default):** Single JSON object with these top-level keys:
- `schema_version` - Always "1.0"
- `kind` - Always "SuperPage"
- `truncated` - Boolean, whether output was truncated
- `diagnostics` - Object with `cycle_count`, `has_cycles`, `component_count`, `expression_count`
- `version` - Page version string
- `theme` - Page theme string
- `params` - Array of page parameters
- `sources` - Array of data sources (models)
- `components` - Array of components
- `expressions` - Array of expressions with parsed refs
- `dependency_order` - Topological sort of component IDs
- `cycles` - Detected cycles (empty if none)
- `next_queries` - Suggested follow-up queries

### Project-Level Graph Queries (require `--project-dir`)

All project-level queries require `--project-dir` to locate the graph database:

```bash
# Build/update graph database
metadata-checker --project-dir /path/to/project --build-graph

# Query model relationships
metadata-checker --project-dir /path/to/project --query-model model1

# Query page dependencies
metadata-checker --project-dir /path/to/project --query-page "page:app/合同管理/销售合同.spg"

# Query cross-file relations
metadata-checker --project-dir /path/to/project --query-cross "page:app/A.spg" "page:app/B.spg"

# Expand DataFlow subgraph
metadata-checker --project-dir /path/to/project --query-dataflow dataflow_output
```

**`query_model` JSON output** includes `schema_version`. Prefer these fields when reasoning about model usage:
- `readers` - Pages/components/actions that read the model
- `writers` - Pages/components/actions that write the model
- `dataflow_inputs` - Input tables read by this DataFlow (outgoing `DataflowInput`)
- `dataflow_outputs` - Physical tables produced by this DataFlow (outgoing `OutputsTo`)
- `produced_by` - DataFlows/apps that produce this physical table (incoming `OutputsTo`)
- `consumed_by_dataflows` - DataFlows that consume this table as input (incoming `DataflowInput`)
- `upstream_dependencies` / `downstream_outputs` - Legacy compatibility fields; do not use them as the primary lineage semantics

For each reader/writer/lineage entry:
- `page` / `page_id` - Source page name and ID
- `component_or_action` - Component or action name
- `node_id` - Full node ID in graph
- `node_type` - Node type (Component, Action, etc.)
- `edge_type` - Relationship type (Reads, Writes, ActionWrites, DataflowInput, OutputsTo)
- `field_path` - Field path if applicable
- `source_file` - Source file path

## Important Constraints

1. `--project-dir` is **required** for all project-level queries (`--query-model`, `--query-page`, `--query-cross`, `--query-dataflow`). Without it, the tool exits with an error.
2. `--human` and `--interactive` both enter REPL mode.
3. When `--priority` is used with `--non-human` (default), priority analysis is merged into the main JSON output under the `priority_analysis` field.
4. Component IDs in expressions use the full path format: `comp:app/page.spg|component_id`.
5. Page node IDs use normalized relative paths: `page:app/page.spg`.

## Examples

```bash
# Parse single file, JSON output
metadata-checker page.spg

# Parse single file, enter interactive REPL
metadata-checker page.spg --human

# Parse with priority analysis (human)
metadata-checker page.spg --human --priority

# Query specific component
metadata-checker page.spg --query input3 --priority

# Interactive REPL
metadata-checker page.spg --interactive

# Build graph for project
metadata-checker --project-dir /path/to/project --build-graph

# Query model (JSON output)
metadata-checker --project-dir /path/to/project --query-model model1

# Query model (human output)
metadata-checker --project-dir /path/to/project --query-model model1 --human
```
