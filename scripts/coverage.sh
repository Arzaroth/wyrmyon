#!/usr/bin/env bash
# Line coverage over the workspace, via cargo-llvm-cov. The CLI tests run the
# built binaries, and their runs count too. crates/testkit is test
# infrastructure and is left out.
#
#   scripts/coverage.sh              summary, then the files with the most
#                                    uncovered lines
#   scripts/coverage.sh --check      fail below WYRMYON_COVERAGE_FLOOR (90)
#   scripts/coverage.sh --html       write and open target/llvm-cov/html
#   scripts/coverage.sh --lcov       write target/llvm-cov/lcov.info
#   scripts/coverage.sh -- ARGS      everything after -- goes to cargo-llvm-cov
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TOP="${WYRMYON_COVERAGE_TOP:-15}"
FLOOR="${WYRMYON_COVERAGE_FLOOR:-90}"
IGNORE='crates/testkit/'

if [[ -t 1 ]]; then B="\033[0;34m"; G="\033[0;32m"; Y="\033[0;33m"; R="\033[0;31m"; Z="\033[0m"; else B=""; G=""; Y=""; R=""; Z=""; fi
info()    { printf '%b\n' "${B}$*${Z}"; }
success() { printf '%b\n' "${G}$*${Z}"; }
warn()    { printf '%b\n' "${Y}$*${Z}"; }
fail()    { printf '%b\n' "${R}$*${Z}" >&2; }

MODE=summary
EXTRA=()
while [[ $# -gt 0 ]]; do
  case "$1" in
  --check) MODE=check ;;
  --html) MODE=html ;;
  --lcov) MODE=lcov ;;
  --) shift; EXTRA=("$@"); break ;;
  -h | --help)
    sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'
    exit 0
    ;;
  *)
    fail "unknown option: $1 (pass cargo-llvm-cov flags after --)"
    exit 1
    ;;
  esac
  shift
done

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  fail "cargo-llvm-cov not found."
  warn "  cargo install cargo-llvm-cov"
  exit 1
fi

if ! find "$(rustc --print sysroot)" -name 'llvm-profdata*' -print -quit 2>/dev/null | grep -q .; then
  fail "llvm-tools-preview is missing from the active toolchain."
  warn "  rustup component add llvm-tools-preview"
  exit 1
fi

cd "$REPO_DIR"
COMMON=(--workspace --locked --ignore-filename-regex "$IGNORE")

case "$MODE" in
check)
  info "Running coverage, floor ${FLOOR}% of lines..."
  cargo llvm-cov "${COMMON[@]}" --summary-only --fail-under-lines "$FLOOR" "${EXTRA[@]+"${EXTRA[@]}"}"
  success "Line coverage is at or above ${FLOOR}%."
  ;;
html)
  info "Running coverage (html)..."
  cargo llvm-cov "${COMMON[@]}" --html --open "${EXTRA[@]+"${EXTRA[@]}"}"
  ;;
lcov)
  info "Running coverage (lcov)..."
  cargo llvm-cov "${COMMON[@]}" --lcov --output-path target/llvm-cov/lcov.info "${EXTRA[@]+"${EXTRA[@]}"}"
  success "Report at target/llvm-cov/lcov.info"
  ;;
summary)
  info "Running coverage..."
  cargo llvm-cov "${COMMON[@]}" --summary-only "${EXTRA[@]+"${EXTRA[@]}"}"
  echo
  info "Most uncovered lines:"
  cargo llvm-cov report --ignore-filename-regex "$IGNORE" --summary-only |
    awk -v top="$TOP" '
      /^[a-zA-Z].*%/ && $1 != "TOTAL" {
        seen++
        if ($9 + 0 > 0) {
          cover = $10; sub(/%$/, "", cover)
          rows[n++] = sprintf("  %6d uncovered  %6.2f%%  %s", $9, cover, $1)
        }
      }
      END {
        if (seen == 0) { print "  (no rows parsed - llvm-cov summary format changed?)"; exit 1 }
        if (n == 0) { print "  (nothing uncovered)"; exit 0 }
        for (i = 0; i < n; i++)
          for (j = i + 1; j < n; j++)
            if ((rows[j] + 0) > (rows[i] + 0)) { t = rows[i]; rows[i] = rows[j]; rows[j] = t }
        for (i = 0; i < n && i < top; i++) print rows[i]
      }'
  ;;
esac
