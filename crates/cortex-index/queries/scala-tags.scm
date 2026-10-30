; Cortex's tags query for Scala. The query that ships with the Scala grammar reads kinds outside the
; tags format (`@definition.object`, `@definition.variable`), so it is not used.

; Methods annotated as tests by JUnit or TestNG: `@Test`, `@ParameterizedTest`, `@org.junit.Test`,
; anywhere. ScalaTest's and MUnit's tests are strings passed to calls, not definitions.
((function_definition
   (annotation
     name: [(type_identifier) @_annotation
            (stable_type_identifier (type_identifier) @_annotation)])
   name: (identifier) @name) @definition.test
 (#any-of? @_annotation "Test" "ParameterizedTest" "RepeatedTest" "TestFactory" "TestTemplate"))

; Classes, case classes, objects and enums are classes; traits are interfaces; a package object is
; a module; type definitions are types.
(class_definition
  name: (identifier) @name) @definition.class
(object_definition
  name: (identifier) @name) @definition.class
(enum_definition
  name: (identifier) @name) @definition.class
(trait_definition
  name: (identifier) @name) @definition.interface
(package_object
  name: (identifier) @name) @definition.module
(type_definition
  name: (type_identifier) @name) @definition.type

; Functions in the body of a class, object or trait are methods, abstract ones included, and so is
; an extension method in an object, as any function directly in a class is; so are the functions
; of a `given` instance, which no class holds. Every other function, at the top of a file or
; inside another, is a function. An operator is named by its symbol (`def +`), read where it has
; a body, which makes it a method in a class or an object; an auxiliary constructor is a method
; named `this`, as calls name it.
(template_body
  [(function_definition name: (identifier) @name)
   (function_declaration name: (identifier) @name)] @definition.method)
(with_template_body
  [(function_definition name: (identifier) @name)
   (function_declaration name: (identifier) @name)] @definition.method)
(function_definition
  name: [(identifier) (operator_identifier)] @name) @definition.function

; Constants: a `val` of one name in an object or a package object, or at the top of a file.
(object_definition
  body: (template_body
    (val_definition
      pattern: (identifier) @name) @definition.constant))
(package_object
  body: (template_body
    (val_definition
      pattern: (identifier) @name) @definition.constant))
(compilation_unit
  (val_definition
    pattern: (identifier) @name) @definition.constant)

; Calls: plain or generic (`fee(x)`, `f[Int](x)`), on a member (`order.pay()`), by name between two
; operands (`xs map f`), and `new Order(1)`, the class plain, generic or qualified, before any
; traits mixed in (`new Worker(task) with Logging`), which the new object inherits: the second
; pattern reads every type of the mixture, the class's name being the call's already.
(call_expression
  function: [(identifier) @name
             (generic_function function: (identifier) @name)]) @reference.call
(call_expression
  function: [(field_expression field: (identifier) @name)
             (generic_function function: (field_expression field: (identifier) @name))]) @reference.call
(infix_expression
  operator: (identifier) @name) @reference.call
(instance_expression
  [(type_identifier) @name
   (generic_type type: (type_identifier) @name)
   (generic_type type: (stable_type_identifier (type_identifier) @name))
   (stable_type_identifier (type_identifier) @name)
   (compound_type
     .
     [(type_identifier) @name
      (generic_type type: (type_identifier) @name)
      (applied_constructor_type
        [(type_identifier) @name
         (generic_type type: (type_identifier) @name)])])]) @reference.call
(instance_expression
  (compound_type
    [(type_identifier) @name
     (generic_type type: (type_identifier) @name)])) @reference.inherit

; Bases: the class and traits after `extends`, `with` or a comma, plain, generic or qualified. The
; grammar gives each of them the same field, which a pattern can match only on the first, so the
; types go unnamed, as the traits mixed in at `new` do.
(extends_clause
  [(type_identifier) @name
   (generic_type type: (type_identifier) @name)
   (generic_type type: (stable_type_identifier (type_identifier) @name))
   (stable_type_identifier (type_identifier) @name)]) @reference.inherit

; Every other type name is a use of the type: in parameters, results, values, type arguments and
; annotations.
(type_identifier) @name @reference.type
