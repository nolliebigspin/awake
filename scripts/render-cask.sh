#!/usr/bin/env bash
# Render the Homebrew cask for the tray app.
# Usage: scripts/render-cask.sh <version> <sha256-of-universal-dmg> <owner/repo>
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <version> <sha256> <owner/repo>" >&2
  exit 2
fi

template="$(dirname "$0")/../packaging/homebrew/awake-tray.rb.tmpl"
sed -e "s|@VERSION@|$1|g" -e "s|@SHA256@|$2|g" -e "s|@REPO@|$3|g" "$template"
