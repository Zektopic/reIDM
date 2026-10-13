#!/usr/bin/env bash
# Usage: tools/decomp.sh <binary> <outfile>   (headless Ghidra full-program decompile)
set -euo pipefail
G=${GHIDRA_HOME:-$HOME/.config/ghidra/ghidra_12.1.4_PUBLIC}
HERE=$(cd "$(dirname "$0")" && pwd)
P=$(mktemp -d)
"$G/support/analyzeHeadless" "$P" proj -import "$1" -scriptPath "$HERE/ghidra" -postScript DecompAll.java "$2" -deleteProject
rm -rf "$P"
