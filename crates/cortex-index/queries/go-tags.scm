; Cortex's patterns for Go, compiled before the tags query that ships with the Go grammar: where a
; pattern here and one of the grammar's find the same name, this one wins.

; Go's built-in types belong to no repository, so their names are not recorded as uses.
((type_identifier) @ignore
  (#any-of? @ignore
    "any" "bool" "byte" "comparable" "complex64" "complex128" "error" "float32" "float64"
    "int" "int8" "int16" "int32" "int64" "rune" "string"
    "uint" "uint8" "uint16" "uint32" "uint64" "uintptr"))

; Methods, named with the type they belong to: `func (s *Shop) Pay()` is `Shop.Pay`.
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [(type_identifier) @qualifier
             (pointer_type (type_identifier) @qualifier)
             (generic_type type: (type_identifier) @qualifier)
             (pointer_type (generic_type type: (type_identifier) @qualifier))]))
  name: (field_identifier) @name) @definition.method

; Structs are classes and interfaces are interfaces; other named types stay types, as the
; grammar's query reads them.
(type_spec name: (type_identifier) @name type: (struct_type)) @definition.class
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface

; Type aliases: `type ID = string`.
(type_alias name: (type_identifier) @name) @definition.type

; Constants declared at the top of a package, alone or in a group: `const Limit = 10`.
(source_file
  (const_declaration
    (const_spec name: (identifier) @name) @definition.constant))
