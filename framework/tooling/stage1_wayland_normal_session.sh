#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${GPUI_STAGE1_ARTIFACT_DIR:?GPUI_STAGE1_ARTIFACT_DIR is required}"
binary="${GPUI_STAGE1_BINARY:?GPUI_STAGE1_BINARY is required}"
story_binary="${GPUI_STAGE1_STORY_BINARY:?GPUI_STAGE1_STORY_BINARY is required}"
validation_profile="${GPUI_STAGE1_VALIDATION_PROFILE:?GPUI_STAGE1_VALIDATION_PROFILE is required}"
: "${WAYLAND_DISPLAY:?WAYLAND_DISPLAY is required}"

python3 tooling/stage1_readiness.py \
  --timeout-seconds 30 \
  --log "$artifact_dir/weston-normal.readiness.log" \
  -- wayland-info
python3 tooling/stage1_watchdog.py \
  --timeout-seconds 30 \
  --stdout "$artifact_dir/vulkaninfo.log" \
  --stderr "$artifact_dir/vulkaninfo.stderr.log" \
  --log "$artifact_dir/vulkaninfo.watchdog.log" \
  -- vulkaninfo --summary
grep -Eiq 'lavapipe|llvmpipe' "$artifact_dir/vulkaninfo.log"

# StoryApp integration evidence: the same bounded watchdog, inside this
# session's compositor/display, writing into the job's existing artifact
# directory. Native handle, renderer, clipboard, input, and accessibility
# evidence stays owned by the conformance scenarios above.
run_story_smoke() {
  GPUI_STAGE1_STORY_EVIDENCE_PATH="$artifact_dir/story-smoke.jsonl" \
    python3 tooling/stage1_watchdog.py \
      --timeout-seconds 180 \
      --stdout "$artifact_dir/story-smoke.stdout.log" \
      --stderr "$artifact_dir/story-smoke.stderr.log" \
      --log "$artifact_dir/story-smoke.watchdog.log" \
      -- "$story_binary" --smoke
  python3 tooling/stage1_watchdog.py \
    --timeout-seconds 30 \
    --stdin "$artifact_dir/story-smoke.jsonl" \
    --stdout "$artifact_dir/story-smoke.validation.stdout.log" \
    --stderr "$artifact_dir/story-smoke.validation.stderr.log" \
    --log "$artifact_dir/story-smoke.validation.watchdog.log" \
    -- "$binary" --validate story-smoke --profile "$validation_profile"
}

run_scenario() {
  local scenario="$1"
  local expected_exit_code="$2"
  python3 tooling/stage1_watchdog.py \
    --timeout-seconds 120 \
    --expected-exit-code "$expected_exit_code" \
    --stdout "$artifact_dir/$scenario.jsonl" \
    --stderr "$artifact_dir/$scenario.stderr.log" \
    --log "$artifact_dir/$scenario.watchdog.log" \
    -- "$binary" --scenario "$scenario"
  python3 tooling/stage1_watchdog.py \
    --timeout-seconds 30 \
    --stdin "$artifact_dir/$scenario.jsonl" \
    --stdout "$artifact_dir/$scenario.validation.stdout.log" \
    --stderr "$artifact_dir/$scenario.validation.stderr.log" \
    --log "$artifact_dir/$scenario.validation.watchdog.log" \
    -- "$binary" --validate "$scenario" --profile "$validation_profile"
}

run_scenario lifecycle-clean 0
run_scenario lifecycle-startup-failure 2
run_scenario lifecycle-background-quit 0
run_scenario window-cycle 0
run_scenario menu-command 0
run_scenario interaction-contracts 0
run_story_smoke
