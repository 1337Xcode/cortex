; Cortex's tags query for Markdown; the grammar ships none.

; Headings, each a module named by its text as written. A heading spans itself alone, from its
; `#` marks to the end of its text, or from its text to the end of the `=` or `-` that underline
; it: the definition's node is the marks or the underline, so that the span stops where the
; heading does, while the grammar's node for a heading in a block quote runs on into the next
; line's `>`. Headings do not nest: the grammar gives an underlined heading no section, so the
; sections it builds for headings written with `#` marks are not the document's. A heading whose
; text is only `#` marks, or ends with a run of them after a space, is not read: it is empty, or
; the grammar keeps its closing marks in its text.
((atx_heading
   [(atx_h1_marker) (atx_h2_marker) (atx_h3_marker) (atx_h4_marker) (atx_h5_marker)
    (atx_h6_marker)] @definition.module
   heading_content: (inline) @name.prose)
 (#not-match? @name.prose "(^| )#+$"))
(setext_heading
  heading_content: (paragraph (inline) @name.prose)
  [(setext_h1_underline) (setext_h2_underline)] @definition.module)
