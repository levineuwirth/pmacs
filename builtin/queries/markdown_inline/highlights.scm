; Markdown (inline) highlights — tree-sitter-md 0.5.3's own query
; (`HIGHLIGHT_QUERY_INLINE`; the crate is MIT, the query credits
; nvim-treesitter) reconciled onto pmacs' recognized capture set
; (src/highlight.rs), as the block overlay beside it is
; (builtin/queries/markdown/highlights.scm, whose header says why the
; captures are renamed rather than themed; #310). The block grammar injects
; this one into every paragraph and heading, so a heading's emphasis
; merges over the heading's face.
;
; Every capture below is an exact entry of `Theme::default_dark`.
; Remapped, by role:
;   @text.literal          -> @string       (code spans, link titles)
;   @text.emphasis         -> @parameter    (the theme's one italic face
;                                            with no color)
;   @text.strong           -> @keyword      (bold; the theme has no bold
;                                            face without a color)
;   @text.uri              -> @constant     (link destinations, autolinks;
;                                            LaTeX's urls)
;   @text.reference        -> @constant     (link labels and text, image
;                                            descriptions; LaTeX's refs)
;   @string.escape         -> @string       (backslash escapes, hard line
;                                            breaks, as the theme already
;                                            painted them)
;   @punctuation.delimiter -> @punctuation  (the theme's plain entry: a
;                                            user theme can style it, the
;                                            default paints nothing, so an
;                                            emphasis or code span keeps
;                                            its face over its delimiters)
; Upstream's query has no `@none`. Node names are upstream's, unchanged.

[
  (code_span)
  (link_title)
] @string

[
  (emphasis_delimiter)
  (code_span_delimiter)
] @punctuation

(emphasis) @parameter

(strong_emphasis) @keyword

[
  (link_destination)
  (uri_autolink)
] @constant

[
  (link_label)
  (link_text)
  (image_description)
] @constant

[
  (backslash_escape)
  (hard_line_break)
] @string

(image
  [
    "!"
    "["
    "]"
    "("
    ")"
  ] @punctuation)

(inline_link
  [
    "["
    "]"
    "("
    ")"
  ] @punctuation)

(shortcut_link
  [
    "["
    "]"
  ] @punctuation)

; NOTE (upstream): extension not enabled by default
; (wiki_link ["[" "|" "]"] @punctuation.delimiter)
