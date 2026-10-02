#!/bin/bash
# E7h review 2: scripts/grammar-fuzz-needed driven by real commits, each on
# the reviewed head in a scratch worktree (unsigned, never pushed), as the
# workflow drives it: --base the PR's base, --head the commit (or the merge
# commit GitHub tests). Usage: tests/e7h_review2/decide_cases.sh WORKTREE BASE
set -euo pipefail
wt=$1 base=$2
cd "$wt"
git checkout -q --detach "$base"
commit() { git -c commit.gpgsign=false -c user.name=probe -c user.email=probe@invalid commit -q -am "$1" 2>/dev/null || git -c commit.gpgsign=false -c user.name=probe -c user.email=probe@invalid commit -q -m "$1"; git rev-parse HEAD; }
case_() { # name, then an edit run in the worktree
    name=$1; shift
    git checkout -q --detach "$base"
    "$@"
    head=$(commit "$name")
    printf '%-34s %s\n' "$name" "$(scripts/grammar-fuzz-needed --base "$base" --head "$head" | tr '\n' ' ')"
}
lock_json() { sed -i '/^name = "tree-sitter-json"$/{n;s/0\.24\.8/0.24.7/;n;n;s/4d727acca406c0020cffc6cf35516764f36c8e3dc4408e5ebe2cb35a947ec471/2cb886676cc805cb37bc756ee2f5188cbf56a6476eb79c734178b8a86dd7e6fa/}' Cargo.lock; git diff --stat | tail -1 >&2; }
lock_serde() { sed -i '/^name = "serde"$/{n;s/1\.0\.228/1.0.227/}' Cargo.lock; }
vendor_edit() { echo '/* probe */' >> vendor/tree-sitter-bash/src/scanner.c; }
queries_edit() { echo '; probe' >> builtin/queries/latex/highlights.scm; }
docs_edit() { echo 'probe' >> docs/divergences.md; }
build_edit() { echo '// probe' >> build/strict_aliasing.rs; }
syntax_move() { mkdir -p src/syntax && git mv src/syntax.rs src/syntax/mod.rs; }
vendor_move() { git mv vendor/tree-sitter-bash/src/scanner.c vendor/tree-sitter-bash/scanner.c.moved; }
case_ "Cargo.lock: grammar 0.24.8->0.24.7" lock_json
case_ "Cargo.lock: serde only" lock_serde
case_ "vendor/ scanner edit" vendor_edit
case_ "builtin/queries/ edit" queries_edit
case_ "docs only" docs_edit
case_ "build/strict_aliasing.rs only" build_edit
case_ "src/syntax.rs -> src/syntax/mod.rs" syntax_move
case_ "vendor/ file moved within" vendor_move
# the merge commit a pull_request run checks out: HEAD^1 is the base
git checkout -q --detach "$base"; docs_edit; d=$(commit docs-head)
git checkout -q --detach "$base"; m=$(git -c commit.gpgsign=false -c user.name=probe -c user.email=probe@invalid merge -q --no-ff --no-edit "$d" >/dev/null && git rev-parse HEAD)
printf '%-34s %s\n' "merge of a docs-only head" "$(scripts/grammar-fuzz-needed --base "$base" --head "$m" | tr '\n' ' ')"
git checkout -q --detach "$base"; vendor_edit; v=$(commit vendor-head)
git checkout -q --detach "$base"; docs_edit; b2=$(commit moved-base)
m=$(git -c commit.gpgsign=false -c user.name=probe -c user.email=probe@invalid merge -q --no-ff --no-edit "$v" >/dev/null && git rev-parse HEAD)
printf '%-34s %s\n' "merge of a vendor head, base moved" "$(scripts/grammar-fuzz-needed --base "$b2" --head "$m" | tr '\n' ' ')"
printf '%-34s %s\n' "all-zero base (new ref)" "$(scripts/grammar-fuzz-needed --base 0000000000000000000000000000000000000000 | tr '\n' ' ')"
printf '%-34s %s\n' "base not in clone" "$(scripts/grammar-fuzz-needed --base 1111111111111111111111111111111111111111 | tr '\n' ' ')"
git checkout -q --detach "$base"
