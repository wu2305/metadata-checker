#!/bin/bash
set -euo pipefail
cd /workspace
echo "COMMIT_BEFORE=$(git rev-parse HEAD)"
git pull --ff-only
cargo fmt
if ! git diff --quiet; then
  git add -A
  git -c user.name="wu2305" -c user.email="wu2305790321@gmail.com" commit -F - <<'EOF'
style: apply cargo fmt to M59 A1b batch

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
  git push origin HEAD
  echo "FMT_COMMIT=$(git rev-parse HEAD)"
else
  echo "FMT_COMMIT=none"
fi
cargo fmt --check
echo "FMT_CHECK_OK"
cargo test --features cli-local \
  --test m59_a1b_component_value_ref_tests \
  --test m59_b4_value_trace_utf8_tests \
  --test superpage_tests \
  --test core_feature_tests \
  --test boundary_error_tests \
  --test edge_case_tests \
  --test additional_tests \
  --test priority_tests \
  --test m58_3_pr2_shape_recursion_tests
echo "TARGETED_TESTS_OK"
cargo check --benches
echo "BENCHES_OK"
cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown
echo "WASM_OK"
echo "REMOTE_CI_OK"
