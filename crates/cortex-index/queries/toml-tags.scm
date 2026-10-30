; Cortex's tags query for TOML; the grammar ships none.

; Keys at the top of a file, each a constant spanning its value, its header the key: `title =
; "shop"`, `version.major = 1`. A key is read as written, a quoted one with its quotes, which the
; grammar leaves outside every token.
(document
  (pair
    [(bare_key) (dotted_key) (quoted_key)] @name.prose
    .
    (_) @body) @definition.constant)

; Tables, each a constant spanning its keys, its header the table's line: `[package]`,
; `[[bin]]`. The first key ends the header, so that a comment before it is not part of it; a
; table with no keys is all header.
(table
  [(bare_key) (dotted_key) (quoted_key)] @name.prose
  (pair) @body) @definition.constant
(table
  [(bare_key) (dotted_key) (quoted_key)] @name.prose) @definition.constant
(table_array_element
  [(bare_key) (dotted_key) (quoted_key)] @name.prose
  (pair) @body) @definition.constant
(table_array_element
  [(bare_key) (dotted_key) (quoted_key)] @name.prose) @definition.constant
