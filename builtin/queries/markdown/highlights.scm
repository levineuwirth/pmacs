; Markdown (block) highlights — tree-sitter-md 0.5.3's own query
; (`HIGHLIGHT_QUERY_BLOCK`; the crate is MIT, the query credits
; nvim-treesitter) reconciled onto
; pmacs' recognized capture set (src/highlight.rs), the LaTeX overlay's
; pattern (builtin/queries/latex/highlights.scm; #310).
;
; The crate's query speaks nvim-treesitter's older vocabulary, and the
; default theme resolves none of its `@text.*` names (there is no `text`
; entry), so every heading, link and code block painted plain. The theme
; is not the outlier: every other bundled query speaks the recognized set
; but tree-sitter-make 1.1.1's, whose `@text.danger`, `@text.warning` and
; `@text.note` mark the arguments of `$(error)`, `$(warning)` and
; `$(info)`, which a `text` entry would repaint. So the overlay renames
; the captures rather than teaching the theme a second vocabulary. Every
; capture below is an exact entry of `Theme::default_dark`, never a name
; that reaches one by dropping dotted segments.
;
; Remapped, by role, after the LaTeX overlay where it has one:
;   @text.title           -> @keyword.control  (heading text; LaTeX's heading)
;   @punctuation.special  -> @keyword          (#, =, - of a heading; LaTeX's
;                                               sectioning command)
;   @punctuation.special  -> @operator         (list markers, thematic breaks,
;                                               block quote markers)
;   @text.literal         -> @string           (code blocks, link titles;
;                                               LaTeX's math)
;   @text.uri             -> @constant         (link destinations; LaTeX's urls)
;   @text.reference       -> @constant         (link labels; LaTeX's refs)
;   @string.escape        -> @string           (as the theme already painted it,
;                                               by its dropped segment)
;
; @none is explicitly no face, not a name that falls through. Upstream puts
; `@text.literal` over a whole fenced block and `@none` over its content:
; the content is the injected language's to paint, or nobody's. pmacs merges
; a narrower capture over a wider one and has no capture that clears, so a
; `@none` left in place would fall through and the content would keep the
; literal face (a fence without a language painted green, a fence's Rust
; identifiers green beneath its keywords). So the literal face goes on the
; fence lines alone, the delimiters and the info string, and no pattern
; paints `code_fence_content`. Upstream's `@punctuation.delimiter` on the
; delimiters painted nothing in pmacs (the theme's `punctuation` is plain),
; so the fence kept the literal face, and still does.
;
; Node names are upstream's, unchanged: `Query::new` compiling this against
; the bundled grammar is the node-name gate (src/syntax.rs).

(atx_heading
  (inline) @keyword.control)

(setext_heading
  (paragraph) @keyword.control)

[
  (atx_h1_marker)
  (atx_h2_marker)
  (atx_h3_marker)
  (atx_h4_marker)
  (atx_h5_marker)
  (atx_h6_marker)
  (setext_h1_underline)
  (setext_h2_underline)
] @keyword

[
  (link_title)
  (indented_code_block)
] @string

(fenced_code_block
  (fenced_code_block_delimiter) @string)

(fenced_code_block
  (info_string) @string)

(link_destination) @constant

(link_label) @constant

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (thematic_break)
] @operator

[
  (block_continuation)
  (block_quote_marker)
] @operator

(backslash_escape) @string
