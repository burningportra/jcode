#!/usr/bin/env bash
# Benchmark harness for graph-ai `jcode-hkd`: measures input tokens + steps
# before/after the code_query pipeline on 5 fixed tasks.
#
# Usage: scripts/bench_codegraph.sh [--run]
#   Without --run: prints the task definitions and measurement protocol
#   (deterministic, no LLM calls — documents the gate).
#   With --run: executes code_query pipelines against the current repo via
#   a debug harness (requires a built binary; manual step).
#
# Gate (plan 8): >=30% input-token reduction on the fixed set, or documented why not.

set -euo pipefail

TASKS=(
  "find-symbol|search for symbol definition then outline the file"
  "blast-radius|dependents + cochanges for a widely-imported file"
  "trace-imports|follow three levels of imports from an entry file"
  "co-change|list files that change with a given file"
  "multi-file-edit-prep|outline all dependents before renaming a symbol"
)

echo "=== codegraph benchmark harness (jcode-hkd) ==="
echo "Tasks: ${#TASKS[@]}"
for t in "${TASKS[@]}"; do
  echo "  - ${t%%|*}: ${t#*|}"
done
echo
echo "Protocol:"
echo "  1. BEFORE: run each task with agentgrep+read only; record input tokens + steps."
echo "  2. AFTER: run each task with code_query pipelines; record input tokens + steps."
echo "  3. Gate: mean input-token reduction >= 30% or document why not in docs/plans/PLAN_GRAPH_AI.md."
echo "  4. Token counting: provider-reported input tokens per turn (usage events),"
echo "     summed over the task's turns. Steps: tool calls executed."
echo
echo "Prompt-map gate (same harness): with agents.codegraph_map=true, verify the"
echo "map block is <=2k tokens (build_map_block MAP_CHAR_BUDGET) and rank-stable"
echo "across two builds (byte-identical ordering)."
echo

if [[ "${1:-}" == "--run" ]]; then
  echo "Automated run: not yet wired to a live agent loop."
  echo "Run manually: enable JCODE_CODEGRAPH_MAP=1 and compare turn token counts."
  exit 2
fi
