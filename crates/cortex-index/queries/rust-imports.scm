; Each path a Rust file brings in with `use`, and each crate with `extern crate`. Paths nested
; more than one brace deep (`use a::{b::{c, d}}`) are not read.

; `use serde;` and `use serde as s;`: the module itself.
(use_declaration
  argument: (identifier) @import.module) @import
(use_declaration
  argument: (use_as_clause
    path: (identifier) @import.module
    alias: (identifier) @import.alias)) @import

; `use std::collections::HashMap;` and `use a::b as c;`: a name from a module.
(use_declaration
  argument: (scoped_identifier
    path: (_) @import.module
    name: (identifier) @import.name)) @import
(use_declaration
  argument: (use_as_clause
    path: (scoped_identifier
      path: (_) @import.module
      name: (identifier) @import.name)
    alias: (identifier) @import.alias)) @import

; `use crate::models::*;`: every public name.
(use_declaration
  argument: (use_wildcard (_) @import.module) @import.all) @import

; `use std::io::{self, Read as R, Write};`: the module for `self`, and each name in the braces.
(use_declaration
  argument: (scoped_use_list
    path: (_) @import.module
    list: (use_list (self)))) @import
(use_declaration
  argument: (scoped_use_list
    path: (_) @import.module
    list: (use_list (identifier) @import.name))) @import
(use_declaration
  argument: (scoped_use_list
    path: (_) @import.module
    list: (use_list
      (use_as_clause
        path: (identifier) @import.name
        alias: (identifier) @import.alias)))) @import

; `extern crate log;` and `extern crate log as logging;`
(extern_crate_declaration
  name: (identifier) @import.module
  alias: (identifier)? @import.alias) @import
