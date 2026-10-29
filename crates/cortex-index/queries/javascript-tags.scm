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

; Constants declared at the top of a module, exported or not: `const LIMIT = 10`. One holding a
; function is a function, as the grammar's own patterns read it.
(program
  (lexical_declaration
    kind: "const"
    (variable_declarator name: (identifier) @name) @definition.constant))
(program
  (export_statement
    declaration: (lexical_declaration
      kind: "const"
      (variable_declarator name: (identifier) @name) @definition.constant)))

; Components used in JSX: `<Widget />` and `<Layout.Header>`. Lower-case names are HTML elements.
((jsx_opening_element name: (identifier) @name @reference.call)
  (#match? @name "^[A-Z]"))
((jsx_self_closing_element name: (identifier) @name @reference.call)
  (#match? @name "^[A-Z]"))
(jsx_opening_element
  name: (member_expression property: (property_identifier) @name) @reference.call)
(jsx_self_closing_element
  name: (member_expression property: (property_identifier) @name) @reference.call)
