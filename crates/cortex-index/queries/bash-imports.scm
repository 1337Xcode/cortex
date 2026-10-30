; Each script a script includes, `source ./lib/common.sh` or `. ./util.sh`, by a literal path,
; bare or in double quotes. A path built from a variable names none. Nor does an option (`--`), a
; home directory (`~/`), a glob (`*`, `?`) or a path with a backslash, which the shell would read
; or rewrite before opening the file; in quotes such a path is literal, but rare, and not read.
((command
   name: (command_name (word) @_include)
   .
   argument: [(word) @import.module
              (string . (string_content) @import.module .)]) @import
 (#any-of? @_include "source" ".")
 (#not-match? @import.module "^[-~]|[*?\\\\]"))
