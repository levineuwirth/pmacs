#!/bin/bash
# E7h review 2: pmacs's compiled build script (build.rs, c7f5e56's refusal)
# run directly under the environment cargo gives a build script, for builds
# this laptop cannot start with cargo: a cross build (no second rustup
# target is installed here), with zig as the cross C compiler. Each case
# prints the script's exit and the refusal's first line, if any.
# Usage: tests/e7h_review2/guard_env.sh BUILD_SCRIPT OUTDIR
set -u
bs=$1 out=$2
mkdir -p "$out"
zcc="$out/zig-aarch64-cc"
printf '#!/bin/sh\nexec zig cc -target aarch64-linux-gnu "$@"\n' > "$zcc"; chmod +x "$zcc"
run() { # label, then env assignments
    label=$1; shift
    rm -rf "$out/o"; mkdir -p "$out/o"
    env -i PATH="$PATH" HOME="$HOME" OUT_DIR="$out/o" OPT_LEVEL=3 DEBUG=false PROFILE=release \
        CARGO_MANIFEST_DIR="$(pwd)" CARGO_PKG_NAME=pmacs PMACS_GIT_HASH=review2 \
        HOST=x86_64-unknown-linux-gnu "$@" "$bs" > "$out/$label.out" 2> "$out/$label.err"
    st=$?
    printf '%-58s exit %s  %s\n' "$label" "$st" "$(head -c 150 "$out/$label.err" | head -n 1)"
}
native="TARGET=x86_64-unknown-linux-gnu CARGO_CFG_TARGET_ARCH=x86_64 CARGO_CFG_TARGET_OS=linux CARGO_CFG_TARGET_ENV=gnu CARGO_CFG_TARGET_VENDOR=unknown CARGO_CFG_TARGET_POINTER_WIDTH=64 CARGO_CFG_TARGET_ENDIAN=little CARGO_CFG_TARGET_FEATURE=fxsr,sse,sse2"
cross="TARGET=aarch64-unknown-linux-gnu CARGO_CFG_TARGET_ARCH=aarch64 CARGO_CFG_TARGET_OS=linux CARGO_CFG_TARGET_ENV=gnu CARGO_CFG_TARGET_VENDOR=unknown CARGO_CFG_TARGET_POINTER_WIDTH=64 CARGO_CFG_TARGET_ENDIAN=little CARGO_CFG_TARGET_FEATURE=neon CC_aarch64_unknown_linux_gnu=$zcc"
# .cargo/config.toml's [env], as cargo passes it from the root:
cfg="HOST_CFLAGS=-fno-strict-aliasing TARGET_CFLAGS=-fno-strict-aliasing"
# shellcheck disable=SC2086
{
run "native, from the root (config's [env])" $native $cfg
run "native, no config (cargo install --git)" $native
run "native, user CFLAGS=-O2 -march=native, from the root" $native $cfg "CFLAGS=-O2 -march=native"
run "native, user CFLAGS=-fstrict-aliasing, from the root" $native $cfg CFLAGS=-fstrict-aliasing
run "native, CFLAGS_<target>=-fstrict-aliasing, from the root" $native $cfg CFLAGS_x86_64-unknown-linux-gnu=-fstrict-aliasing
run "native, no config, CFLAGS=-fno-strict-aliasing (remedy)" $native CFLAGS=-fno-strict-aliasing
run "cross aarch64 (zig cc), from the root (config's [env])" $cross $cfg
run "cross aarch64 (zig cc), no config" $cross
run "cross aarch64 (zig cc), no config, CFLAGS remedy" $cross CFLAGS=-fno-strict-aliasing
run "cross aarch64 (zig cc), user CFLAGS=-O2, from the root" $cross $cfg CFLAGS=-O2
run "cross aarch64, no C cross compiler installed, from the root" TARGET=aarch64-unknown-linux-gnu CARGO_CFG_TARGET_ARCH=aarch64 CARGO_CFG_TARGET_OS=linux CARGO_CFG_TARGET_ENV=gnu CARGO_CFG_TARGET_VENDOR=unknown CARGO_CFG_TARGET_POINTER_WIDTH=64 CARGO_CFG_TARGET_ENDIAN=little $cfg
}
