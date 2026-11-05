# Third-party material

pmacs is under the GNU General Public License, version 3 ([LICENSE](LICENSE)).
The material below comes from others and stays under its own license. A release
archive carries the fonts' and the Apache-2.0 license texts under `licenses/`.

| Material | Where | License | License text |
|---|---|---|---|
| JetBrains Mono, compiled into `pmacs-gpu` | `pmacs-gpu/fonts/JetBrainsMono-Regular.ttf` | SIL Open Font License 1.1 | `pmacs-gpu/fonts/OFL.txt` |
| Latin Modern Math, compiled into `pmacs-gpu` | `pmacs-gpu/fonts/latinmodern-math.otf` | GUST Font License | `pmacs-gpu/fonts/GUST-FONT-LICENSE.txt` |
| Tree-sitter grammars for Bash, HTML, Markdown, Python and YAML | `vendor/tree-sitter-*/` | MIT | each directory's `LICENSE` |
| The Markdown highlight queries, from tree-sitter-md 0.5.3, which credits nvim-treesitter; changed for pmacs | `builtin/queries/markdown/highlights.scm`, `builtin/queries/markdown_inline/highlights.scm` | MIT | `vendor/tree-sitter-md/LICENSE` |
| The LaTeX highlight query, from nvim-treesitter; changed for pmacs | `builtin/queries/latex/highlights.scm` | Apache-2.0 | `licenses/Apache-2.0.txt` |
| The Lean 4 abbreviation table, generated from vscode-lean4 | `builtin/runtime/lean_abbrev.lua` | Apache-2.0 | `licenses/Apache-2.0.txt` |

Rust dependencies, other grammars among them, are not in this repository. Each
is under the license its own manifest names; `cargo metadata` lists them.

The fonts under `pmacs-gpu/fonts/test/` are original to pmacs and under its
license (`pmacs-gpu/fonts/test/LICENSE.txt`).
