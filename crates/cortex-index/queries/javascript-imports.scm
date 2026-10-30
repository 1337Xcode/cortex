; Each module a JavaScript file imports, with what it takes from it and the name it binds.

; `import React from "react"`: the default export, bound to a name.
(import_statement
  (import_clause (identifier) @import.default @import.alias)
  source: (string (string_fragment) @import.module)) @import

; `import { useState, useMemo as memo } from "react"`. TypeScript's grammar reads `default` in
; braces as an identifier, JavaScript's as a keyword, so the default is told apart by its text.
((import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @import.name
        alias: (identifier)? @import.alias)))
  source: (string (string_fragment) @import.module)) @import
  (#not-eq? @import.name "default"))

; `import { default as Base } from "react"`: the default export again, under a name of the file's
; choosing.
((import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: [(identifier) "default"] @import.default
        alias: (identifier) @import.alias)))
  source: (string (string_fragment) @import.module)) @import
  (#eq? @import.default "default"))

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
; and `export * as helpers from "./helpers"`. A name may be re-exported as the default:
; `export { Shape as default } from "./shape"`.
((export_statement
  (export_clause
    (export_specifier
      name: (identifier) @import.name
      alias: [(identifier) "default"]? @import.alias))
  source: (string (string_fragment) @import.module)) @import
  (#not-eq? @import.name "default"))
; The default export re-exported: `export { default } from "./widget"` and
; `export { default as Card } from "./card"`.
((export_statement
  (export_clause
    (export_specifier
      name: [(identifier) "default"] @import.default
      alias: (identifier)? @import.alias))
  source: (string (string_fragment) @import.module)) @import
  (#eq? @import.default "default"))
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
