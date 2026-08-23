#!/usr/bin/env node
// Redact a literal token from a file in place. Used by every `.cnb.yml`
// stage/endStage script that needs it (they don't share a shell process,
// so each one shells out to this file instead of carrying its own copy
// of the redaction logic). Literal split/join, not a regex replace: the
// token may contain regex metacharacters.

import { existsSync, readFileSync, writeFileSync } from "node:fs";

const [, , file, token] = process.argv;

if (!token) {
  process.exit(0);
}
if (!file || !existsSync(file)) {
  process.exit(0);
}

const text = readFileSync(file, "utf8");
if (text.includes(token)) {
  writeFileSync(file, text.split(token).join("[REDACTED_CNB_TOKEN]"));
}
