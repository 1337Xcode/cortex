; Each class, function or constant a PHP file imports with `use`: `use App\Models\Item;` takes
; `Item` from `App\Models`, `use Item;` takes a name from the global namespace, `as` renames, and a
; group, `use App\Models\{Line, Receipt as Slip, Sub\Item};`, takes each name, qualified or not,
; from its prefix. `use function` and `use const` read the same way.
(namespace_use_declaration
  (namespace_use_clause
    (qualified_name
      prefix: (namespace_name) @import.module
      (name) @import.name
      .)
    alias: (name)? @import.alias)) @import
(namespace_use_declaration
  (namespace_use_clause
    .
    (name) @import.module
    alias: (name)? @import.alias)) @import
(namespace_use_declaration
  (namespace_name) @import.module
  body: (namespace_use_group
    (namespace_use_clause
      .
      [(name) (qualified_name)] @import.name
      alias: (name)? @import.alias))) @import

; Each file it includes by a literal path, quoted either way, or by a path relative to its own
; folder, either one perhaps in parentheses: `require 'config.php';`, `require("config.php");`,
; `require_once(__DIR__ . '/helpers.php');`. A path built otherwise, as from `dirname(__FILE__)`,
; or with a variable or an escape inside its quotes, names no file.
([
  (require_expression
    [(string . (string_content) @import.module .)
     (encapsed_string . (string_content) @import.module .)
     (parenthesized_expression
       [(string . (string_content) @import.module .)
        (encapsed_string . (string_content) @import.module .)
        (binary_expression
          left: (name) @_folder
          right: [(string . (string_content) @import.module .)
                  (encapsed_string . (string_content) @import.module .)])])
     (binary_expression
       left: (name) @_folder
       right: [(string . (string_content) @import.module .)
               (encapsed_string . (string_content) @import.module .)])])
  (require_once_expression
    [(string . (string_content) @import.module .)
     (encapsed_string . (string_content) @import.module .)
     (parenthesized_expression
       [(string . (string_content) @import.module .)
        (encapsed_string . (string_content) @import.module .)
        (binary_expression
          left: (name) @_folder
          right: [(string . (string_content) @import.module .)
                  (encapsed_string . (string_content) @import.module .)])])
     (binary_expression
       left: (name) @_folder
       right: [(string . (string_content) @import.module .)
               (encapsed_string . (string_content) @import.module .)])])
  (include_expression
    [(string . (string_content) @import.module .)
     (encapsed_string . (string_content) @import.module .)
     (parenthesized_expression
       [(string . (string_content) @import.module .)
        (encapsed_string . (string_content) @import.module .)
        (binary_expression
          left: (name) @_folder
          right: [(string . (string_content) @import.module .)
                  (encapsed_string . (string_content) @import.module .)])])
     (binary_expression
       left: (name) @_folder
       right: [(string . (string_content) @import.module .)
               (encapsed_string . (string_content) @import.module .)])])
  (include_once_expression
    [(string . (string_content) @import.module .)
     (encapsed_string . (string_content) @import.module .)
     (parenthesized_expression
       [(string . (string_content) @import.module .)
        (encapsed_string . (string_content) @import.module .)
        (binary_expression
          left: (name) @_folder
          right: [(string . (string_content) @import.module .)
                  (encapsed_string . (string_content) @import.module .)])])
     (binary_expression
       left: (name) @_folder
       right: [(string . (string_content) @import.module .)
               (encapsed_string . (string_content) @import.module .)])])] @import
 (#eq? @_folder "__DIR__"))
