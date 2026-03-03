#!/usr/bin/env bash
# Example: Scan a repository and display drift analysis
#
# Usage:
#   ./examples/scan_and_analyze.sh /path/to/repo

set -euo pipefail

REPO_PATH="${1:-.}"

echo "=== MorphArch: Scan & Analyze ==="
echo "Repository: $REPO_PATH"
echo

# Step 1: Scan the repository
echo "--- Scanning ---"
morpharch scan "$REPO_PATH" -n 200

# Step 2: Show drift trend
echo
echo "--- Drift Trend ---"
morpharch list-drift

# Step 3: Analyze HEAD
echo
echo "--- HEAD Analysis ---"
morpharch analyze

echo
echo "=== Done ==="
