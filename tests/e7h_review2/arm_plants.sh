#!/bin/sh
# E7h review 2: show each planted defect firing, or not, under each fuzz
# arm's own C flags, through review 1's driver (tests/e7h_review1/drv.c),
# before the harness sees any of it. The plant is lua_arm_plant.patch applied
# to tree-sitter-lua 0.5.0; LUA_SRC names that copy's src/. The arms'
# grammar compiles, as scripts/fuzz-grammars builds them under the fuzz
# profile (-O3, the crate's -std=c11, .cargo/config.toml's flag first):
#   ubsan        $CC -fno-strict-aliasing -fsanitize=address,undefined
#                -fno-sanitize-recover=undefined
#   asan-strict  $CC -fno-strict-aliasing -fsanitize=address -fstrict-aliasing
#   tysan        clang -fno-strict-aliasing -fsanitize=type (the runtime
#                uninstrumented, as the arm builds it)
# Usage: [CC=gcc] [TYSAN_CC=clang] [MODES="none crash ..."] LUA_SRC=DIR \
#        tests/e7h_review2/arm_plants.sh [OUTDIR]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cc=${CC:-gcc}
tcc=${TYSAN_CC:-clang}
out=${1:-${TMPDIR:-/tmp}/e7h-review2-arms}
reg=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -n 1)
rt="$reg/tree-sitter-0.26.8"
lua=${LUA_SRC:?LUA_SRC names the planted tree-sitter-lua src/}
mkdir -p "$out/in"
printf -- '--[[ x @Z y ]]\nlocal a = 1\n' > "$out/in/trigger.lua"
printf -- '--[[ x @Y y ]]\nlocal a = 1\n' > "$out/in/control.lua"
echo "$($cc --version | head -n 1); $($tcc --version | head -n 1)"
build() { # tag grammar-cc runtime-cc link-cc grammar-flags...
    tag=$1 gcc_=$2 rcc=$3 lcc=$4; shift 4
    d="$out/obj/$tag"; mkdir -p "$d"
    "$rcc" -O3 -g -fno-strict-aliasing ${RT_SAN:-} -c -I"$rt/include" -I"$rt/src" "$rt/src/lib.c" -o "$d/lib.o"
    "$gcc_" -O3 -g -std=c11 -fno-strict-aliasing "$@" -c -I"$lua" "$lua/parser.c" -o "$d/parser.o"
    "$gcc_" -O3 -g -std=c11 -fno-strict-aliasing "$@" -c -I"$lua" "$lua/scanner.c" -o "$d/scanner.o"
    "$rcc" -O3 -g ${RT_SAN:-} -DLANG=tree_sitter_lua -c -I"$rt/include" "$here/../e7h_review1/drv.c" -o "$d/drv.o"
    "$lcc" ${LINK_SAN:-} "$d"/*.o -o "$out/$tag"
}
RT_SAN="-fsanitize=address,undefined -fno-sanitize-recover=undefined" LINK_SAN="$RT_SAN" \
    build ubsan "$cc" "$cc" "$cc" -fsanitize=address,undefined -fno-sanitize-recover=undefined
RT_SAN="-fsanitize=address" LINK_SAN="$RT_SAN" \
    build asan-strict "$cc" "$cc" "$cc" -fsanitize=address -fstrict-aliasing
RT_SAN= LINK_SAN="-fsanitize=type" \
    build tysan "$tcc" "$tcc" "$tcc" -fsanitize=type
export ASAN_OPTIONS=detect_leaks=0:abort_on_error=1
export UBSAN_OPTIONS=halt_on_error=1:abort_on_error=1:print_stacktrace=1
for mode in ${MODES:-none crash overflow pun alias}; do
    line="$mode:"
    for arm in ubsan asan-strict tysan; do
        log="$out/$mode-$arm.log"
        set +e
        PMACS_PLANT=$mode "$out/$arm" "$out/in/trigger.lua" cold > "$log" 2>&1
        st=$?
        set -e
        # TypeSanitizer reports and carries on: a report of one type over
        # another is the arm's finding, as the harness files it.
        verdict=clean
        [ "$st" = 0 ] || verdict="exit $st"
        if grep -q "ERROR: TypeSanitizer" "$log"; then
            verdict="$verdict, $(grep -m1 -o 'with type [^(]* accesses an existing object of type [^ (]*' "$log" || echo 'TySan report')"
        fi
        if grep -q "ERROR: AddressSanitizer" "$log"; then
            verdict="$verdict, $(grep -m1 -o 'AddressSanitizer: [a-z-]*' "$log")"
        fi
        if grep -q "runtime error:" "$log"; then
            verdict="$verdict, $(grep -m1 -o 'runtime error: [a-z ]*' "$log")"
        fi
        line="$line  $arm=[$verdict]"
    done
    echo "$line"
done
# the control input under every arm and mode: nothing fires without @Z
for mode in crash overflow pun alias; do
    for arm in ubsan asan-strict tysan; do
        PMACS_PLANT=$mode "$out/$arm" "$out/in/control.lua" cold > "$out/control-$mode-$arm.log" 2>&1 \
            || echo "control fired: $mode $arm"
        ! grep -q "ERROR: TypeSanitizer" "$out/control-$mode-$arm.log" || echo "control TySan: $mode $arm"
    done
done
echo "controls done"
