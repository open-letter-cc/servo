#!/usr/bin/env bash
# Copyright 2026 Open Letter, Inc.
# SPDX-License-Identifier: LicenseRef-OpenLetter-Tools
# Export-surface gate for an ELF or Mach-O payload. The Windows job has the same
# logic in PowerShell against the PE export directory; both read the lists in
# this directory.
#
# usage: check.sh <shared library> <floor>
#
# NM overrides the nm binary, for a cross-compiled payload whose format the
# host's nm may not read (an NDK's llvm-nm for an Android library, say).
set -euo pipefail

lib="$1"
floor="$2"
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NM="${NM:-nm}"

names() { grep -vE '^[[:space:]]*(#|$)' "$dir/$1"; }

case "$lib" in
  *.dylib)
    # nm -g on Mach-O prefixes every C symbol with an underscore.
    exports=$("$NM" -g --defined-only "$lib" | awk '{print $3}' | sed 's/^_//' | grep -E '^servo_' | sort -u)
    ;;
  *)
    exports=$("$NM" -D --defined-only "$lib" | awk '{print $3}' | grep -E '^servo_' | sort -u)
    ;;
esac

count=$(printf '%s\n' "$exports" | grep -c . || true)
echo "$(basename "$lib") exports $count servo_* symbols"

has() { printf '%s\n' "$exports" | grep -qx "$1"; }

status=0
missing_required=()
missing_optional=()
for s in $(names required.txt); do has "$s" || missing_required+=("$s"); done
for s in $(names optional.txt); do has "$s" || missing_optional+=("$s"); done

if ((${#missing_required[@]})); then
  echo "::error::$(basename "$lib") is missing ${#missing_required[@]} REQUIRED symbol(s); the embedder's library load fails outright: ${missing_required[*]}"
  status=1
fi
if ((${#missing_optional[@]})); then
  echo "::error::$(basename "$lib") is missing ${#missing_optional[@]} promised optional symbol(s): ${missing_optional[*]}"
  status=1
fi

ungated=()
for s in $(names planned.txt); do
  if has "$s" && ! { names required.txt; names optional.txt; } | grep -qx "$s"; then
    ungated+=("$s")
  fi
done
if ((${#ungated[@]})); then
  echo "::error::$(basename "$lib") exports ${#ungated[@]} planned symbol(s) this gate does not check: ${ungated[*]}. Classify each in required.txt or optional.txt in the same commit that implements it."
  status=1
fi

if ((count < floor)); then
  echo "::error::$(basename "$lib") exports only $count servo_* symbols, expected at least $floor"
  status=1
fi

printf '%s\n' "$exports" | sed 's/^/  /'
exit $status
