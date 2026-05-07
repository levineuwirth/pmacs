;; pmacs audit-lint rule set, v1.0 (T M7.9).
;;
;; Each pattern below corresponds to one entry in
;; `src/audit/rules.rs::DEFAULT_RULES` *by index*. Reordering or
;; inserting patterns here without updating the Rust table is a bug;
;; the unit test `rules_table_aligns_with_query_file` catches it.
;;
;; The outermost node of every pattern is captured as `@violation`.
;; The audit engine reports that node's byte range as the finding's
;; snippet and its row/column as the location.
;;
;; Style: queries deliberately match exact identifier text rather
;; than relying on naming heuristics. A package author who routes
;; through a local alias (`local f = io.open` then `f("x", "w")`)
;; bypasses these rules; that is acceptable for v1.0 -- the rule
;; set is a reviewable contract, not an undefeatable barrier.
;; Whole-program data-flow analysis is reserved for the full-moon
;; tier (see TRANSITION-M7.md, T M7.9 section).

;; ---------------------------------------------------------------------------
;; Rule 0: no-private-surface (require)
;;
;; Forbids `require("pmacs._internal.X")` and `require("pmacs.core.X")`.
;; ---------------------------------------------------------------------------
((function_call
   (identifier) @fn
   (arguments (string (string_content) @arg)))
 (#eq? @fn "require")
 (#match? @arg "^pmacs\\.(_internal|core)(\\.|$)")) @violation

;; ---------------------------------------------------------------------------
;; Rule 1: no-private-surface (identifier)
;;
;; Forbids any identifier prefixed with `_pmacs_internal_` or `_core_`.
;; ---------------------------------------------------------------------------
((identifier) @id
 (#match? @id "^(_pmacs_internal_|_core_)")) @violation

;; ---------------------------------------------------------------------------
;; Rule 2: no-ffi-call
;;
;; Forbids `ffi.cdef`, `ffi.load`, `ffi.metatype` (LuaJIT FFI surface).
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "ffi")
 (#match? @m "^(cdef|load|metatype)$")) @violation

;; ---------------------------------------------------------------------------
;; Rule 3: no-package-loadlib
;;
;; Forbids `package.loadlib(...)` (Lua 5.4 native-library equivalent of
;; the LuaJIT FFI surface).
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "package")
 (#eq? @m "loadlib")) @violation

;; ---------------------------------------------------------------------------
;; Rule 4: no-package-cpath-mutation
;;
;; Forbids assignment to `package.cpath` (would extend the C-loader
;; search path, defeating no-FFI).
;; ---------------------------------------------------------------------------
((assignment_statement
   (variable_list (dot_index_expression (identifier) @t (identifier) @f)))
 (#eq? @t "package")
 (#eq? @f "cpath")) @violation

;; ---------------------------------------------------------------------------
;; Rule 5: no-debug-sethook
;;
;; Forbids `debug.sethook(...)`. The pmacs cancellation hook (T M7.8)
;; is installed once at VM init; a package overwriting it disables
;; cooperative cancellation editor-wide.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "debug")
 (#eq? @m "sethook")) @violation

;; ---------------------------------------------------------------------------
;; Rule 6: no-debug-setmetatable
;;
;; Forbids `debug.setmetatable(...)` (bypasses normal metatable rules,
;; can re-skin third-party tables silently).
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "debug")
 (#eq? @m "setmetatable")) @violation

;; ---------------------------------------------------------------------------
;; Rule 7: no-rawget-rawset-on-globals
;;
;; Forbids `rawget(_G, ...)` and `rawset(_G, ...)` (escape from any
;; per-package _ENV sandboxing into the shared global table).
;; ---------------------------------------------------------------------------
((function_call
   (identifier) @fn
   (arguments . (identifier) @arg1))
 (#match? @fn "^(rawget|rawset)$")
 (#eq? @arg1 "_G")) @violation

;; ---------------------------------------------------------------------------
;; Rule 8: no-setfenv-getfenv
;;
;; Forbids `setfenv` and `getfenv` (Lua 5.1 / LuaJIT environment
;; manipulation; absent from Lua 5.4 by design but still callable on
;; LuaJIT). Static analysis cannot tell whether the target stack
;; frame belongs to another package, so any call is flagged.
;; ---------------------------------------------------------------------------
((function_call
   (identifier) @fn)
 (#match? @fn "^(setfenv|getfenv)$")) @violation

;; ---------------------------------------------------------------------------
;; Rule 9: no-fs-mutation-io-open-write
;;
;; Warns on `io.open(<path>, "w"|"a"|"w+"|"a+"|"r+"|"wb"|...)`. The
;; mode string contains 'w' or 'a' or '+' for any non-read-only mode.
;; Read-only `io.open(path, "r")` is allowed; omitting the mode
;; defaults to read and is also allowed.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m)
   (arguments . (_) (string (string_content) @mode)))
 (#eq? @t "io")
 (#eq? @m "open")
 (#match? @mode "[wa+]")) @violation

;; ---------------------------------------------------------------------------
;; Rule 10: no-fs-mutation-os
;;
;; Warns on `os.remove(...)` and `os.rename(...)`.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "os")
 (#match? @m "^(remove|rename)$")) @violation

;; ---------------------------------------------------------------------------
;; Rule 11: no-process-spawn-io
;;
;; Warns on `io.popen(...)`.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "io")
 (#eq? @m "popen")) @violation

;; ---------------------------------------------------------------------------
;; Rule 12: no-process-spawn-os
;;
;; Warns on `os.execute(...)`.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression (identifier) @t (identifier) @m))
 (#eq? @t "os")
 (#eq? @m "execute")) @violation

;; ---------------------------------------------------------------------------
;; Rule 13: no-process-spawn-pmacs
;;
;; Warns on `pmacs.process.spawn(...)`. Legitimate users (REPL,
;; magit, LSP launcher) declare process access in their manifest;
;; anything else is a finding.
;; ---------------------------------------------------------------------------
((function_call
   (dot_index_expression
     (dot_index_expression (identifier) @t1 (identifier) @t2)
     (identifier) @m))
 (#eq? @t1 "pmacs")
 (#eq? @t2 "process")
 (#eq? @m "spawn")) @violation

;; ---------------------------------------------------------------------------
;; Rule 14: reach-around-require
;;
;; Info-level: detects `require("name.submodule")` calls where the
;; require target is dotted (looks like cross-package access). The
;; auditor classifies each finding against the target package's
;; declared `exports`. Absent a known-package registry, this rule
;; fires once per dotted require and a human verifies. With the
;; registry (`pmacs-audit --known-packages packages.toml`) the
;; engine promotes confirmed cross-package private accesses to
;; Error-level findings.
;;
;; The `pmacs.X` namespaces (the host's own surface) are excluded
;; because rules 0/1 already handle private-surface access there.
;; ---------------------------------------------------------------------------
((function_call
   (identifier) @fn
   (arguments (string (string_content) @arg)))
 (#eq? @fn "require")
 (#match? @arg "\\.")
 (#not-match? @arg "^pmacs(\\.|$)")) @violation
