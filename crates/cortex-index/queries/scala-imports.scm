; Each name a Scala file imports, from the package or object its path names: `import shop.models.Item`
; takes `Item` from `shop.models`; `import shop.util._` and Scala 3's `import shop.util.*` take
; every name; `import shop.support.{Money, Clock => Time, _}` takes each selected name, renamed or
; not, and every other one, as Scala 3's `as` renames too. The grammar writes a path as a run of
; identifiers with no node around it, so a module is captured from its first identifier
; (`@import.module`) to its last (`@import.module.end`); it gives every identifier of the path the
; same field, which a pattern can match only on the first, so the identifiers go unnamed. It also
; runs the clauses of `import a.B, c.D` together into one path, so an import of several clauses is
; not read; nor is `given`, which imports only given instances.
((import_declaration
   .
   (identifier) @import.module
   (identifier) @import.module.end
   .
   (identifier) @import.name
   .) @import
 (#not-match? @import "^[^{]*,|}\\s*,"))
((import_declaration
   .
   (identifier) @import.module
   .
   (identifier) @import.name
   .) @import
 (#not-match? @import "^[^{]*,|}\\s*,"))
(import_declaration
  .
  (identifier) @import.module
  .) @import
((import_declaration
   .
   (identifier) @import.module
   (identifier) @import.module.end
   .
   (namespace_wildcard) @import.all) @import
 (#not-match? @import "^[^{]*,|}\\s*,")
 (#not-eq? @import.all "given"))
((import_declaration
   .
   (identifier) @import.module
   .
   (namespace_wildcard) @import.all) @import
 (#not-match? @import "^[^{]*,|}\\s*,")
 (#not-eq? @import.all "given"))
((import_declaration
   .
   (identifier) @import.module
   (identifier) @import.module.end
   .
   [(namespace_selectors
      [(identifier) @import.name
       (arrow_renamed_identifier
         name: (identifier) @import.name
         alias: (identifier) @import.alias)
       (as_renamed_identifier
         name: (identifier) @import.name
         alias: (identifier) @import.alias)
       (namespace_wildcard) @import.all])
    (as_renamed_identifier
      name: (identifier) @import.name
      alias: (identifier) @import.alias)]) @import
 (#not-match? @import "^[^{]*,|}\\s*,")
 (#not-eq? @import.all "given"))
((import_declaration
   .
   (identifier) @import.module
   .
   [(namespace_selectors
      [(identifier) @import.name
       (arrow_renamed_identifier
         name: (identifier) @import.name
         alias: (identifier) @import.alias)
       (as_renamed_identifier
         name: (identifier) @import.name
         alias: (identifier) @import.alias)
       (namespace_wildcard) @import.all])
    (as_renamed_identifier
      name: (identifier) @import.name
      alias: (identifier) @import.alias)]) @import
 (#not-match? @import "^[^{]*,|}\\s*,")
 (#not-eq? @import.all "given"))
