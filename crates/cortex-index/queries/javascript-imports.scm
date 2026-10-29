; Each module a JavaScript file imports, with what it takes from it and the name it binds.

; `import React from "react"`: the default export, bound to a name.
(import_statement
  (import_clause (identifier) @import.default @import.alias)
  source: (string (string_fragment) @import.module)) @import

; `import { useState, useMemo as memo, default as Base } from "react"`
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: [(identifier) "default"] @import.name
        alias: (identifier)? @import.alias)))
  source: (string (string_fragment) @import.module)) @import

; `import * as api from "./api"`
(import_statement
  (import_clause (namespace_import (identifier) @import.alias) @import.all)
  source: (string (string_fragment) @import.module)) @import

; `import "./styles.css"`: the module alone, loaded for what loading it does.
(import_statement
  "import"
  .
  source: (string (string_fragment) @import.module)) @import

; Re-exports: `export { format, parse as parseOrder } from "./format"`, `export * from "./types"`
; and `export * as helpers from "./helpers"`.
(export_statement
  (export_clause
    (export_specifier
      name: [(identifier) "default"] @import.name
      alias: [(identifier) "default"]? @import.alias))
  source: (string (string_fragment) @import.module)) @import
(export_statement
  "*" @import.all
  source: (string (string_fragment) @import.module)) @import
(export_statement
  (namespace_export (identifier) @import.alias) @import.all
  source: (string (string_fragment) @import.module)) @import

; `require("fs")` and `import("./lazy")`, with the module written as a plain string. The name a
; `require` is assigned to is not read.
((call_expression
  function: (identifier) @_require
  arguments: (arguments . (string (string_fragment) @import.module) .)) @import
  (#eq? @_require "require"))
(call_expression
  function: (import)
  arguments: (arguments . (string (string_fragment) @import.module))) @import
