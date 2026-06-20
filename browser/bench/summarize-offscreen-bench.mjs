const DEFAULT_TIMING_KEYS = [
  "wasm_init_ms",
  "fixture_load_ms",
  "runtime_load_ms",
  "build_graph_ms",
  "runtime_load_build_graph_ms",
  "analyze_selection_ms",
  "serialize_output_ms",
  "total_ms",
];

function percentile(values, p) {
  if (values.length === 0) {
    return null;
  }
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[index];
}

function aggregateTiming(rows, key) {
  const values = rows
    .map((row) => row.timing?.[key])
    .filter((value) => typeof value === "number" && Number.isFinite(value));
  return {
    p50: percentile(values, 50),
    p95: percentile(values, 95),
    max: values.length > 0 ? Math.max(...values) : null,
  };
}

function dominantStage(timingAgg) {
  const stageKeys = [
    "wasm_init_ms",
    "fixture_load_ms",
    "runtime_load_ms",
    "build_graph_ms",
    "analyze_selection_ms",
    "serialize_output_ms",
  ];
  let dominant = null;
  let dominantP50 = -1;
  for (const key of stageKeys) {
    const p50 = timingAgg[key]?.p50;
    if (typeof p50 === "number" && p50 >= dominantP50) {
      dominant = key;
      dominantP50 = p50;
    }
  }
  return dominant;
}

export function summarizeOffscreenBench(records) {
  const groups = new Map();
  for (const record of records) {
    const key = `${record.scenario_base}::${record.sample}::${record.component_kind}`;
    if (!groups.has(key)) {
      groups.set(key, []);
    }
    groups.get(key).push(record);
  }

  const scenarios = [];
  for (const [key, rows] of groups.entries()) {
    const [scenario, sample, componentKind] = key.split("::");
    const totals = aggregateTiming(rows, "total_ms");
    const timing = {};
    for (const timingKey of DEFAULT_TIMING_KEYS) {
      timing[timingKey] = aggregateTiming(rows, timingKey);
    }
    scenarios.push({
      scenario,
      sample,
      component_kind: componentKind,
      iteration_count: rows.length,
      ok_count: rows.filter((row) => row.ok).length,
      artifact_ready_count: rows.filter((row) => row.artifact_ready).length,
      total_ms: totals,
      timing,
      dominant_stage: dominantStage(timing),
    });
  }

  scenarios.sort((left, right) => {
    const scenarioCmp = left.scenario.localeCompare(right.scenario);
    if (scenarioCmp !== 0) {
      return scenarioCmp;
    }
    const sampleCmp = left.sample.localeCompare(right.sample);
    if (sampleCmp !== 0) {
      return sampleCmp;
    }
    return left.component_kind.localeCompare(right.component_kind);
  });

  const markdownLines = [
    "# Browser Offscreen WASM Bench Summary",
    "",
    "| scenario | sample | component_kind | iterations | total p50 | total p95 | total max | dominant_stage |",
    "|---|---|---|---:|---:|---:|---:|---|",
    ...scenarios.map((row) => (
      `| ${row.scenario} | ${row.sample} | ${row.component_kind} | ${row.iteration_count} | ${row.total_ms.p50 ?? ""} | ${row.total_ms.p95 ?? ""} | ${row.total_ms.max ?? ""} | ${row.dominant_stage ?? ""} |`
    )),
    "",
    "## Stage p50 / p95 / max (ms)",
    "",
  ];

  for (const row of scenarios) {
    markdownLines.push(`### ${row.scenario} / ${row.sample} / ${row.component_kind}`);
    markdownLines.push("");
    markdownLines.push("| stage | p50 | p95 | max |");
    markdownLines.push("|---|---:|---:|---:|");
    for (const timingKey of DEFAULT_TIMING_KEYS) {
      const stats = row.timing[timingKey];
      markdownLines.push(
        `| ${timingKey} | ${stats.p50 ?? ""} | ${stats.p95 ?? ""} | ${stats.max ?? ""} |`,
      );
    }
    markdownLines.push("");
  }

  return {
    schema_version: 1,
    scenario_count: scenarios.length,
    record_count: records.length,
    scenarios,
    markdown: markdownLines.join("\n"),
  };
}
