#!/bin/sh
# E7h review 1: tree-sitter-haskell 0.23.1 on the tree-sitter 0.26.8 runtime,
# built outside cargo with the system compiler, across the two axes that
# decide whether its aliasing UB becomes a memory error:
#   -fno-strict-aliasing or not, and the sanitizer configuration
#   (none, ASan, ASan+UBSan as the fuzz job builds, UBSan).
# Each build parses the owner's two-pragma file and three of E7g's crashers
# cold and retyped a byte at a time (drv.c). Usage:
#   CC=gcc tests/e7h_review1/haskell_matrix.sh [OUTDIR]
# Needs the two crates in cargo's registry (`cargo fetch`).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cc=${CC:-gcc}
out=${1:-${TMPDIR:-/tmp}/e7h-review1-haskell}
reg=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -n 1)
rt="$reg/tree-sitter-0.26.8"
hs="$reg/tree-sitter-haskell-0.23.1/src"
[ -d "$rt" ] && [ -d "$hs" ] || { echo "cargo fetch first: $rt, $hs" >&2; exit 2; }
mkdir -p "$out/in"
printf '{-# LANGUAGE OverloadedStrings #-}\n{-# LANGUAGE ScopedTypeVariables #-}\n' > "$out/in/two.hs"
# three of E7g's six crashers, byte for byte (9 bytes each)
printf '{- #ent \n' > "$out/in/crash-0.hs"
printf '{-   - a\n' > "$out/in/crash-1.hs"
printf -- '-- a\n-- a' > "$out/in/crash-2.hs"
echo "$($cc --version | head -n 1)"
build() { # tag flags...
    tag=$1; shift
    d="$out/obj/$tag"; mkdir -p "$d"
    "$cc" "$@" -c -I"$rt/include" -I"$rt/src" "$rt/src/lib.c" -o "$d/lib.o"
    "$cc" "$@" -c -I"$hs" "$hs/parser.c" -o "$d/parser.o"
    "$cc" "$@" -c -I"$hs" "$hs/scanner.c" -o "$d/scanner.o"
    "$cc" "$@" -DLANG=tree_sitter_haskell -c -I"$rt/include" "$here/drv.c" -o "$d/drv.o"
    "$cc" "$@" "$d"/*.o -o "$out/$tag"
}
for opt in O2 O3; do
    for san in plain asan asan-ubsan ubsan; do
        case $san in
            plain) s= ;;
            asan) s="-fsanitize=address" ;;
            asan-ubsan) s="-fsanitize=address,undefined -fno-sanitize-recover=undefined" ;;
            ubsan) s="-fsanitize=undefined -fno-sanitize-recover=undefined" ;;
        esac
        for alias in strict no-strict; do
            a=; [ $alias = no-strict ] && a=-fno-strict-aliasing
            tag="$opt-$san-$alias"
            # shellcheck disable=SC2086
            build "$tag" -$opt -g -fno-omit-frame-pointer $s $a
            bad=0
            for f in "$out"/in/*; do
                for m in cold retype; do
                    ASAN_OPTIONS=detect_leaks=0 "$out/$tag" "$f" $m > "$out/$tag.log" 2>&1 || bad=$((bad + 1))
                done
            done
            echo "$tag: $bad of 8 runs failed"
        done
    done
done
