; Each file a Ruby file loads by a literal path: `require "json"`, `require_relative "../lib/shop"`
; and `load "tasks/release.rake"`, and the file `autoload :Shop, "shop"` loads on first use. A
; path built at run time, as in `require File.join(dir, "shop")` or `require "#{dir}/shop"`, names
; no file. A method with a receiver, such as `YAML.load`, is another method.
((call
   !receiver
   method: (identifier) @_method
   arguments: (argument_list
     .
     (string
       .
       (string_content) @import.module
       .))) @import
 (#any-of? @_method "require" "require_relative" "load"))
((call
   !receiver
   method: (identifier) @_method
   arguments: (argument_list
     .
     (_)
     .
     (string
       .
       (string_content) @import.module
       .))) @import
 (#eq? @_method "autoload"))
