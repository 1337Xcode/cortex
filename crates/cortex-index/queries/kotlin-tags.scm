; Cortex's tags query for Kotlin; the grammar ships none. The grammar names no field for a body, so
; each definition captures its own (`@body`) for the header to end before it.

; Functions annotated as tests by JUnit, kotlin.test or TestNG: `@Test`, `@ParameterizedTest`,
; `@org.junit.Test`, with arguments or without.
((function_declaration
   (modifiers
     (annotation
       [(user_type (identifier) @_annotation .)
        (constructor_invocation (user_type (identifier) @_annotation .))]))
   name: (identifier) @name
   (function_body)? @body) @definition.test
 (#any-of? @_annotation "Test" "ParameterizedTest" "RepeatedTest" "TestFactory" "TestTemplate"))

; Functions in the body of a class, interface or object are methods, in an anonymous object too,
; where no class holds them; a function directly in an enum is a method as any function directly
; in a class is. Every other function, at the top of a file or inside another function, an
; extension included, is a function.
(class_body
  (function_declaration
    name: (identifier) @name
    (function_body)? @body) @definition.method)
(function_declaration
  name: (identifier) @name
  (function_body)? @body) @definition.function

; Interfaces, `fun interface`s included, and annotation classes are interfaces; classes of every
; other sort, objects and named companion objects are classes.
(class_declaration
  "interface"
  name: (identifier) @name
  (class_body)? @body) @definition.interface
((class_declaration
   (modifiers (class_modifier) @_sort)
   name: (identifier) @name
   (class_body)? @body) @definition.interface
 (#eq? @_sort "annotation"))
; A class or object that delegates to an object and has a body (`: MutableMap<K, V> by m { ... }`):
; the grammar can read the body as a lambda passed to the delegate, which Kotlin does not allow
; there, so that lambda is the body.
(class_declaration
  name: (identifier) @name
  (delegation_specifiers
    (delegation_specifier
      (explicit_delegation
        (call_expression
          (annotated_lambda) @body))))) @definition.class
(object_declaration
  name: (identifier) @name
  (delegation_specifiers
    (delegation_specifier
      (explicit_delegation
        (call_expression
          (annotated_lambda) @body))))) @definition.class
(class_declaration
  name: (identifier) @name
  [(class_body) (enum_class_body)]? @body) @definition.class
(object_declaration
  name: (identifier) @name
  (class_body)? @body) @definition.class
(companion_object
  name: (identifier) @name
  (class_body)? @body) @definition.class

; Type aliases; the grammar puts an alias's name in the field `type`.
(type_alias
  type: (identifier) @name) @definition.type

; Constants: `const val`, at the top of a file, in an object or a companion object.
(property_declaration
  (modifiers (property_modifier))
  (variable_declaration
    (identifier) @name)) @definition.constant

; Where the grammar reads such a body as a lambda, the delegate's name, plain or reached through
; another (`by Holder.m`), is no call. A delegate given arguments (`by Wrapped(m)`) stays one:
; the grammar puts its call inside the one that takes the lambda.
(explicit_delegation
  (call_expression
    (identifier) @ignore
    (annotated_lambda)))
(explicit_delegation
  (call_expression
    (navigation_expression
      (identifier) @ignore
      .)
    (annotated_lambda)))

; Calls: of a function or a constructor (`fee(x)`, `Order(1)`, `listOf<Int>()`, `run { }`), of a
; member (`order.pay()`, `item?.pay()`, `super.toString()`), and infix (`1 to 2`, `a until b`).
(call_expression
  (identifier) @name) @reference.call
(call_expression
  (navigation_expression
    (identifier) @name
    .)) @reference.call
(infix_expression
  .
  (_)
  .
  (identifier) @name) @reference.call

; Bases: the superclass, interfaces implemented, and interfaces delegated (`Logs by Logger()`),
; plain, generic or qualified.
(delegation_specifier
  [(user_type (identifier) @name .)
   (user_type (identifier) @name . (type_arguments) .)
   (constructor_invocation
     [(user_type (identifier) @name .)
      (user_type (identifier) @name . (type_arguments) .)])
   (explicit_delegation
     [(user_type (identifier) @name .)
      (user_type (identifier) @name . (type_arguments) .)])]) @reference.inherit

; Every other use of a type: parameters, results, properties, type arguments and bounds, receivers,
; casts and checks, caught exceptions and annotations, plain, generic, qualified or nullable.
(user_type (identifier) @name .) @reference.type
(user_type (identifier) @name . (type_arguments) .) @reference.type
