; Every other type name in Rust is a use of the type: in fields, parameters, return types and
; generic arguments. Compiled after the grammar's query, so a name it reads as a definition stays
; one. Built-in types such as `u32` and `str` are not type names to the grammar.
(type_identifier) @name @reference.type
