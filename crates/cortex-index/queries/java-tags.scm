; Cortex's patterns for Java, compiled before the tags query that ships with the Java grammar:
; where a pattern here and one of the grammar's find the same name, this one wins.

; Methods annotated as tests by JUnit or TestNG: `@Test`, `@ParameterizedTest`, `@org.junit.Test`.
((method_declaration
   (modifiers
     [(marker_annotation
        name: [(identifier) @_annotation
               (scoped_identifier name: (identifier) @_annotation)])
      (annotation
        name: [(identifier) @_annotation
               (scoped_identifier name: (identifier) @_annotation)])])
   name: (identifier) @name) @definition.test
 (#any-of? @_annotation "Test" "ParameterizedTest" "RepeatedTest" "TestFactory" "TestTemplate"))

; Base classes, plain, generic or qualified: `extends Base<Item>`. The grammar's query reads a
; base class as a use of the class, as it reads `new Base()`.
(superclass
  [(type_identifier) @name
   (generic_type (type_identifier) @name)
   (scoped_type_identifier (type_identifier) @name .)]) @reference.inherit

; Interfaces given with type arguments: `implements Comparable<Order>`. The grammar's query reads
; the plain ones.
(type_list
  (generic_type (type_identifier) @name) @reference.inherit)

; Enums and records are classes, annotation types are interfaces, constructors are methods.
(enum_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
(annotation_type_declaration name: (identifier) @name) @definition.interface
(constructor_declaration name: (identifier) @name) @definition.method

; Constants: `static final` fields, with the modifiers in either order, and every field of an
; interface.
(field_declaration
  (modifiers "static" "final")
  declarator: (variable_declarator name: (identifier) @name) @definition.constant)
(field_declaration
  (modifiers "final" "static")
  declarator: (variable_declarator name: (identifier) @name) @definition.constant)
(constant_declaration
  declarator: (variable_declarator name: (identifier) @name) @definition.constant)
