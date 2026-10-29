; Additions to the tags query that ships with the JavaScript grammar, compiled after it: where a
; pattern here and one of the grammar's find the same name, the grammar's wins.

; Base classes: `class A extends B` and `class A extends models.B`.
(class_heritage
  [(identifier) @name
   (member_expression property: (property_identifier) @name)] @reference.inherit)

; Class fields holding functions, as components often declare their handlers: `handle = () => {}`.
(field_definition
  property: (property_identifier) @name
  value: [(arrow_function) (function_expression)]) @definition.method
