; Each class or member a Java file imports: `import java.util.List;`, the static
; `import static org.junit.Assert.assertEquals;`, and every public name with `import java.util.*;`.
; A named import ends right after its name, which tells it from a wildcard.
(import_declaration
  (scoped_identifier
    scope: (_) @import.module
    name: (identifier) @import.name)
  .
  ";") @import
(import_declaration
  (_) @import.module
  .
  (asterisk) @import.all) @import
