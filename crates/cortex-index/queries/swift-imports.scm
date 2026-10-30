; Each module a Swift file imports, its path as written: `import Foundation`, a submodule
; `import Foundation.NSString`, `@testable import Orders`. An import of one declaration, with its
; kind (`import struct Shop.Money`), takes the declaration's name from the module before it.
(import_declaration
  ["typealias" "struct" "class" "enum" "protocol" "let" "var" "func"]? @_kind
  (identifier) @import.module
  (#not-any-of? @_kind "typealias" "struct" "class" "enum" "protocol" "let" "var" "func")) @import
(import_declaration
  ["typealias" "struct" "class" "enum" "protocol" "let" "var" "func"]
  (identifier
    .
    (simple_identifier) @import.module
    .
    (simple_identifier) @import.name
    .)) @import
(import_declaration
  ["typealias" "struct" "class" "enum" "protocol" "let" "var" "func"]
  (identifier
    .
    (simple_identifier) @import.module
    (simple_identifier) @import.module.end
    .
    (simple_identifier) @import.name
    .)) @import
