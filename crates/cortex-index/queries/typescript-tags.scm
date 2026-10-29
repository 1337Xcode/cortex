; Additions to the tags queries that ship with the JavaScript and TypeScript grammars, compiled
; after them: where a pattern here and one of theirs find the same name, theirs wins.

; Namespaces: `namespace Billing {}`. The grammar's own query reads `module Billing {}`.
(internal_module name: (identifier) @name) @definition.module

; Type aliases and enums: `type Id = string`, `enum Color { Red }`.
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.type

; Class fields holding functions: `private handle = () => {}`.
(public_field_definition
  name: (property_identifier) @name
  value: [(arrow_function) (function_expression)]) @definition.method

; Base classes and implemented interfaces, plain, qualified or generic: `extends Base`,
; `implements Shape, geo.Area`, and the interfaces an interface extends.
(extends_clause
  value: [(identifier) @name
          (member_expression property: (property_identifier) @name)] @reference.inherit)
(implements_clause
  [(type_identifier) @name
   (nested_type_identifier name: (type_identifier) @name)
   (generic_type
     name: [(type_identifier) @name
            (nested_type_identifier name: (type_identifier) @name)])] @reference.inherit)
(extends_type_clause
  type: [(type_identifier) @name
         (nested_type_identifier name: (type_identifier) @name)
         (generic_type
           name: [(type_identifier) @name
                  (nested_type_identifier name: (type_identifier) @name)])] @reference.inherit)

; Every other type name is a use of the type: in annotations, generic arguments, unions and the
; rest. It comes last, so a name read above as a definition or a base stays one. Type parameters
; such as the `T` of `Box<T>` are read as uses too.
(type_identifier) @name @reference.type
