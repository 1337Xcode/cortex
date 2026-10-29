; TypeScript's own form, read after JavaScript's: `import fs = require("fs")` binds the module to a
; name. `import type` and `export type` read as the plain forms do.
(import_statement
  (import_require_clause
    (identifier) @import.alias
    source: (string (string_fragment) @import.module))) @import
