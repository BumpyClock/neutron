#!/bin/bash

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# Check if version argument is provided
new_version=$1
if [ -z "$new_version" ]
then
  echo -e "${RED}${BOLD}Error:${RESET} Version argument is required"
  echo -e "${YELLOW}USAGE:${RESET} ./framework/script/bump-version.sh [VERSION]"
  exit 1
fi

# Keep version changes inside the framework domain.
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd -- "$(cd -- "$script_dir/.." && pwd)"
repo_root=$(git rev-parse --show-toplevel)

# Logging functions
function log_header() {
  local message=$1
  echo ""
  echo -e "${BOLD}${BLUE}╔════════════════════════════════════════════════════════╗${RESET}"
  echo -e "${BOLD}${BLUE}║${RESET}  ${CYAN}${BOLD}$message${RESET}"
  echo -e "${BOLD}${BLUE}╚════════════════════════════════════════════════════════╝${RESET}"
  echo ""
}

function log_step() {
  local step=$1
  local message=$2
  echo -e "${MAGENTA}${BOLD}[$step]${RESET} ${message}"
}

function log_success() {
  local message=$1
  echo -e "${GREEN}${BOLD}✓${RESET} ${message}"
}

function log_info() {
  local message=$1
  echo -e "${CYAN}ℹ${RESET} ${message}"
}

function log_error() {
  local message=$1
  echo -e "${RED}${BOLD}✗${RESET} ${message}"
}

# Refuse to create a release commit from a dirty worktree.
if [ -n "$(git status --porcelain --untracked-files=all)" ]; then
  log_error "Worktree must be clean before a version bump"
  exit 1
fi

if git show-ref --verify --quiet "refs/tags/framework-v$new_version"; then
  log_error "Tag framework-v$new_version already exists"
  exit 1
fi

if ! cargo set-version --help >/dev/null 2>&1; then
  log_error "cargo-edit with cargo set-version is required"
  exit 1
fi

# Update only packages whose manifests belong to framework/.
framework_packages=(
  gpui-component-app
  gpui-component
  gpui-component-macros
  gpui-component-manifest
  gpui-component-storage
  gpui-component-story
  gpui-component-assets
  framework-reqwest-client
  gpui-wry
  gpui-component-conformance
  app_assets
  app_shell
  app_shell_background
  hello_world
  input
  window_title
  dialog_overlay
  webview
  system_monitor
  focus_trap
  framework-xtask
)

# Start release process
log_header "Starting Framework Version Bump for $new_version"

# Step 1: Update framework package versions
log_step "1/4" "Updating framework packages to version ${BOLD}$new_version${RESET}"
for package in "${framework_packages[@]}"; do
  if ! cargo set-version --package "$package" "$new_version"; then
    log_error "Failed to update framework package $package"
    exit 1
  fi
done

if ! python3 - "$new_version" <<'PY'
import re
import sys
from pathlib import Path

version = sys.argv[1]
path = Path("compatibility.toml")
text = path.read_text(encoding="utf-8")
section, separator, remainder = text.partition("[framework]\n")
if not separator:
    raise SystemExit("compatibility.toml has no [framework] section")
body, marker, tail = remainder.partition("\n[gpui]\n")
updated, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{version}"', body, count=1)
if count != 1:
    raise SystemExit("[framework].version is missing or ambiguous")
path.write_text(section + separator + updated + marker + tail, encoding="utf-8")
PY
then
  log_error "Failed to update framework compatibility version"
  exit 1
fi

if ! cargo run --locked -p framework-xtask -- compatibility generate; then
  log_error "Failed to generate compatibility documentation"
  exit 1
fi

if ! cargo run --locked -p framework-xtask -- compatibility check; then
  log_error "Framework compatibility check failed"
  exit 1
fi
log_success "Framework package versions updated successfully"
echo ""

# Step 2: Stage changes
log_step "2/4" "Staging modified files"
if git -C "$repo_root" add -- \
  Cargo.toml Cargo.lock framework/compatibility.toml framework/docs/COMPATIBILITY.md \
  && git -C "$repo_root" add -u -- framework; then
  log_success "Files staged successfully"
else
  log_error "Failed to stage files"
  exit 1
fi
echo ""

# Step 3: Create version commit
log_step "3/4" "Creating version commit"
if git commit -m "chore(release): bump framework to $new_version"; then
  log_success "Commit created: ${BOLD}chore(release): bump framework to $new_version${RESET}"
else
  log_error "Failed to create commit"
  exit 1
fi
echo ""

# Step 4: Provide validation instruction
log_step "4/4" "Preparing validation instruction"
log_info "Run all release gates and exact-commit Stage 1 before tag creation."
log_info "This script did not create a tag, push, or publish a package."
echo ""

# Success message
echo -e "${GREEN}${BOLD}╔════════════════════════════════════════════════════════╗${RESET}"
echo -e "${GREEN}${BOLD}║${RESET}  ${BOLD}🚀 Framework version commit $new_version is ready!${RESET}"
echo -e "${GREEN}${BOLD}║${RESET}  ${GREEN}Run release gates and exact-commit Stage 1 next.${RESET}"
echo -e "${GREEN}${BOLD}║${RESET}  ${GREEN}No tag, remote push, or publication was performed.${RESET}"
echo -e "${GREEN}${BOLD}╚════════════════════════════════════════════════════════╝${RESET}"
echo ""
