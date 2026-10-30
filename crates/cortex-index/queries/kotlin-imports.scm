; Each declaration a Kotlin file imports: `import shop.models.Item` takes `Item` from `shop.models`,
; `as` renames it, and `import shop.util.*` takes every name of `shop.util`. An import of a name
; with no package, `import Orders`, takes it whole. The grammar writes a dotted name as a run of
; identifiers with no node around the package, so the package is captured from its first
; identifier (`@import.module`) to its last (`@import.module.end`). A wildcard follows the dotted
; name as a token of its own, which `"*"? @_wildcard` finds, so that the other patterns leave it out.
(import
  (qualified_identifier
    .
    (identifier) @import.module
    (identifier) @import.module.end
    .
    (identifier) @import.name
    .)
  (identifier)? @import.alias
  "*"? @_wildcard
  (#not-eq? @_wildcard "*")) @import
(import
  (qualified_identifier
    .
    (identifier) @import.module
    .
    (identifier) @import.name
    .)
  (identifier)? @import.alias
  "*"? @_wildcard
  (#not-eq? @_wildcard "*")) @import
(import
  (qualified_identifier
    .
    (identifier) @import.module
    .)
  (identifier)? @import.alias
  "*"? @_wildcard
  (#not-eq? @_wildcard "*")) @import
(import
  (qualified_identifier) @import.module
  "*" @import.all) @import
