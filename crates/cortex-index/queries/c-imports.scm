; Each file a C file includes. `#include "shop/order.h"` gives the path between the quotes, and
; `#include <stdio.h>` the path with its angle brackets, which the grammar keeps as one token: the
; brackets tell a header looked up on the include path from one first looked up beside the file.
; A path a macro gives, as in `#include CONFIG_H`, names no file.
(preproc_include
  path: (string_literal
    .
    (string_content) @import.module
    .)) @import
(preproc_include
  path: (system_lib_string) @import.module) @import
