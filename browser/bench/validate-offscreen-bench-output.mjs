export function validateOffscreenBenchOutput({
  records,
  summary,
  requiredTimingKeys,
}) {
  const errors = [];
  if (!Array.isArray(records) || records.length === 0) {
    errors.push("records must be a non-empty array");
  }
  if (!summary || !Array.isArray(summary.scenarios) || summary.scenarios.length === 0) {
    errors.push("summary.scenarios must be a non-empty array");
  }

  for (const record of records) {
    if (record.ok !== true) {
      errors.push(`record not ok: ${record.scenario} iteration ${record.iteration}`);
    }
    if (record.artifact_ready !== true) {
      errors.push(`artifact not ready: ${record.scenario} iteration ${record.iteration}`);
    }
    for (const key of requiredTimingKeys) {
      const value = record.timing?.[key];
      if (typeof value !== "number" || !Number.isFinite(value)) {
        errors.push(`missing or invalid timing.${key} for ${record.scenario} iteration ${record.iteration}`);
      }
    }
  }

  return {
    ok: errors.length === 0,
    errors,
  };
}
