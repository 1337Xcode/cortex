; Each file a C++ file includes, read as in C: the path between quotes, or a system include's
; path with its angle brackets.
(preproc_include
  path: (string_literal
    .
    (string_content) @import.module
    .)) @import
(preproc_include
  path: (system_lib_string) @import.module) @import
