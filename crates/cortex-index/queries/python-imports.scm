; Each module a Python file imports, with the name it takes from it and the name it binds. A
; `from __future__ import` line is an instruction to the compiler, not an import, and is not read.

; `import a.b` and `import a.b as c`
(import_statement
  name: (dotted_name) @import.module) @import
(import_statement
  name: (aliased_import
    name: (dotted_name) @import.module
    alias: (identifier) @import.alias)) @import

; `from a import b`, `from .a import b as c` and `from a import *`
(import_from_statement
  module_name: (_) @import.module
  name: (dotted_name) @import.name) @import
(import_from_statement
  module_name: (_) @import.module
  name: (aliased_import
    name: (dotted_name) @import.name
    alias: (identifier) @import.alias)) @import
(import_from_statement
  module_name: (_) @import.module
  (wildcard_import) @import.all) @import
