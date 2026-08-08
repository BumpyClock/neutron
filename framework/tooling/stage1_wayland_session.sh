#!/usr/bin/env bash
set -euo pipefail

repo_root="${GPUI_STAGE1_REPO_ROOT:?GPUI_STAGE1_REPO_ROOT is required}"
artifact_dir="${GPUI_STAGE1_ARTIFACT_DIR:?GPUI_STAGE1_ARTIFACT_DIR is required}"
binary="${GPUI_STAGE1_BINARY:?GPUI_STAGE1_BINARY is required}"
validation_profile="linux-wayland-lavapipe"
reader_command="${GPUI_STAGE1_WAYLAND_CLIPBOARD_READER:?GPUI_STAGE1_WAYLAND_CLIPBOARD_READER is required}"
cd "$repo_root"

python3 tooling/stage1_clipboard_harness.py \
  --binary "$binary" \
  --timeout-seconds 120 \
  --stdout "$artifact_dir/clipboard.jsonl" \
  --stderr "$artifact_dir/clipboard.stderr.log" \
  --log "$artifact_dir/clipboard.watchdog.log" \
  --reader-stdout "$artifact_dir/clipboard.reader.stdout.log" \
  --reader-stderr "$artifact_dir/clipboard.reader.stderr.log" \
  --validation-stdout "$artifact_dir/clipboard.validation.stdout.log" \
  --validation-stderr "$artifact_dir/clipboard.validation.stderr.log" \
  --validation-log "$artifact_dir/clipboard.validation.watchdog.log" \
  --validation-profile "$validation_profile" \
  --reader-command "$reader_command"
printf 'passed\n' > "$artifact_dir/clipboard.fixture-result"
