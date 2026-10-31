; Each package a Go file imports, alone or in a group: `import "fmt"`, `import f "fmt"`,
; `import . "fmt"` (every exported name) and `import _ "embed"` (for what loading it does, under
; the blank name, which reaches nothing).
(import_spec
  !name
  path: [(interpreted_string_literal (interpreted_string_literal_content) @import.module)
         (raw_string_literal (raw_string_literal_content) @import.module)]) @import
(import_spec
  name: (package_identifier) @import.alias
  path: [(interpreted_string_literal (interpreted_string_literal_content) @import.module)
         (raw_string_literal (raw_string_literal_content) @import.module)]) @import
(import_spec
  name: (dot) @import.all
  path: [(interpreted_string_literal (interpreted_string_literal_content) @import.module)
         (raw_string_literal (raw_string_literal_content) @import.module)]) @import
(import_spec
  name: (blank_identifier) @import.alias
  path: [(interpreted_string_literal (interpreted_string_literal_content) @import.module)
         (raw_string_literal (raw_string_literal_content) @import.module)]) @import
