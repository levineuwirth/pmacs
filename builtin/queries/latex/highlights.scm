; LaTeX highlights — vendored from nvim-treesitter (Apache-2.0) and reconciled
; onto pmacs' recognized capture set (src/highlight.rs; framing Q#LX2 / F4).
;
; Reconciliation vs the verbatim upstream (previous commit):
;   * @spell / @nospell markers stripped — meaningless to pmacs, and a
;     same-node @nospell risks clobbering the real capture.
;   * The 8 predicate-gated patterns (#eq? / #any-of? / #lua-match?) removed:
;     pmacs evaluates only `#is? local` predicates, so unevaluated they would
;     over-match every generic command (as \if-conditional / emphasis) and
;     every line_comment (as a magic directive). Re-instating them is deferred
;     to predicate support. This also drops the only @markup.italic/@markup.strong
;     uses, so those need no remap.
;   * Fall-through captures remapped onto the recognized set:
;       @module        -> @keyword         (\begin/\end/sectioning commands)
;       @label         -> @type            (environment / theorem names)
;       @markup.heading -> @keyword.control (section-title text)
;       @markup.link*  -> @constant        (labels, refs, citations, urls)
;       @markup.math   -> @string          (inline/displayed formulas)
; Grammar node names are UNCHANGED from upstream — Query::new compiling this
; against the bundled grammar is the node-name compatibility gate (acceptance 3).

; General syntax
(command_name) @function

(caption
  command: _ @function)

; \text, \intertext, \shortintertext, ...
(text_mode
  command: _ @function
  content: (curly_group
    (_) @none))

; Variables, parameters
(placeholder) @variable

(key_value_pair
  key: (_) @variable.parameter
  value: (_))

(curly_group_spec
  (text) @variable.parameter)

(brack_group_argc) @variable.parameter

[
  (operator)
  "="
  "_"
  "^"
] @operator

"\\item" @punctuation.special

(delimiter) @punctuation.delimiter

(math_delimiter
  left_command: _ @punctuation.delimiter
  left_delimiter: _ @punctuation.delimiter
  right_command: _ @punctuation.delimiter
  right_delimiter: _ @punctuation.delimiter)

[
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket ; "(" ")" has no syntactical meaning in LaTeX

; General environments
(begin
  command: _ @keyword
  name: (curly_group_text
    (text) @type))

(end
  command: _ @keyword
  name: (curly_group_text
    (text) @type))

; Definitions and references
(new_command_definition
  command: _ @function.macro)

(old_command_definition
  command: _ @function.macro)

(let_command_definition
  command: _ @function.macro)

(environment_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @type))

(theorem_definition
  command: _ @function.macro
  name: (curly_group_text_list
    (_) @type))

(paired_delimiter_definition
  command: _ @function.macro
  declaration: (curly_group_command_name
    (_) @function))

; NOTE: this grammar cut (codebook 0.6.1, ~Dec 2025) uses distinct
; `curly_group_label`/`curly_group_label_list` nodes for label commands, where
; newer latex-lsp (which nvim's query targets) unified them onto
; `curly_group_text`. Kept grammar-accurate here (acceptance 3 is the gate).
(label_definition
  command: _ @function.macro
  name: (curly_group_label
    (_) @constant))

(label_reference_range
  command: _ @function.macro
  from: (curly_group_label
    (_) @constant)
  to: (curly_group_label
    (_) @constant))

(label_reference
  command: _ @function.macro
  names: (curly_group_label_list
    (_) @constant))

(label_number
  command: _ @function.macro
  name: (curly_group_label
    (_) @constant)
  number: (_) @constant)

(citation
  command: _ @function.macro
  keys: (curly_group_text_list) @constant)

(hyperlink
  command: _ @function
  uri: (curly_group_uri
    (_) @constant))

(glossary_entry_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant))

(glossary_entry_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant))

(acronym_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant))

(acronym_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant))

(color_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant))

(color_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @constant)?)

; Sectioning
(title_declaration
  command: _ @keyword
  options: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(author_declaration
  command: _ @keyword
  authors: (curly_group_author_list
    (author)+ @keyword.control))

(chapter
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(part
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(section
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(subsection
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(subsubsection
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(paragraph
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

(subparagraph
  command: _ @keyword
  toc: (brack_group
    (_) @keyword.control)?
  text: (curly_group
    (_) @keyword.control))

; File inclusion commands
(class_include
  command: _ @keyword.import
  path: (curly_group_path) @string)

(package_include
  command: _ @keyword.import
  paths: (curly_group_path_list) @string)

(latex_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(verbatim_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(import_include
  command: _ @keyword.import
  directory: (curly_group_path) @string.special.path
  file: (curly_group_path) @string.special.path)

(bibstyle_include
  command: _ @keyword.import
  path: (curly_group_path) @string)

(bibtex_include
  command: _ @keyword.import
  paths: (curly_group_path_list) @string.special.path)

(biblatex_include
  "\\addbibresource" @keyword.import
  glob: (curly_group_glob_pattern) @string.regexp)

(graphics_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(svg_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(inkscape_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(tikz_library_import
  command: _ @keyword.import
  paths: (curly_group_path_list) @string)

; Math
[
  (displayed_equation)
  (inline_formula)
] @string

(math_environment
  (_) @string)

; Comments
[
  (line_comment)
  (block_comment)
  (comment_environment)
] @comment
