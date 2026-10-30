; Cortex's tags query for Swift. The query that ships with the Swift grammar tags a method's whole
; class as the method and reads properties with a kind outside the tags format, so it is not used.
; A pattern that looks inside a function declaration costs about a tenth of a second to compile,
; the grammar giving the declaration many forms, so as few patterns as can do so.

; Functions marked as tests by Swift Testing, `@Test` or `@Test("pays the order")`, anywhere, an
; attribute being named by its last part; XCTest's methods named `test...` are tests in test files.
((function_declaration
   (modifiers
     (attribute
       (user_type (type_identifier) @_attribute .)))
   name: (simple_identifier) @name) @definition.test
 (#eq? @_attribute "Test"))

; Classes, structs, enums and actors are classes; protocols are interfaces; type aliases and a
; protocol's associated types are types.
(class_declaration
  declaration_kind: ["class" "struct" "enum" "actor"]
  name: (type_identifier) @name) @definition.class
(protocol_declaration
  name: (type_identifier) @name) @definition.interface
(typealias_declaration
  name: (type_identifier) @name) @definition.type
(associatedtype_declaration
  name: (type_identifier) @name) @definition.type

; An extension adds to a type declared elsewhere. It is a class named after that type, by the last
; part of a qualified name (`extension Shop.Order` is `Order`), so what it declares is qualified as
; the type's own members are: a type and its extensions are definitions of one name. An extension
; of an array, a dictionary or an optional is named as written (`[Int]`).
(class_declaration
  declaration_kind: "extension"
  name: [(user_type (type_identifier) @name .)
         (user_type (type_identifier) @name . (type_arguments) .)
         (array_type) @name
         (dictionary_type) @name
         (optional_type) @name]) @definition.class

; Functions, those in a type or an extension being its methods; a protocol's functions and every
; initializer, named `init` as calls name it, are methods.
(function_declaration
  name: (simple_identifier) @name) @definition.function
(protocol_function_declaration
  name: (simple_identifier) @name) @definition.method
(init_declaration
  name: "init" @name) @definition.method

; Constants: `let` at the top of a file and `static let` in a type, one name to a declaration;
; Swift allows no other modifier of that kind before a `let`.
(source_file
  (property_declaration
    (value_binding_pattern mutability: "let")
    .
    name: (pattern bound_identifier: (simple_identifier) @name)
    .
    (type_annotation)?
    .
    value: (_)
    .) @definition.constant)
(property_declaration
  (modifiers (property_modifier))
  (value_binding_pattern mutability: "let")
  .
  name: (pattern bound_identifier: (simple_identifier) @name)
  .
  (type_annotation)?
  .
  value: (_)
  .) @definition.constant

; Attributes, `@objc` or `@MainActor`, are mostly the compiler's own and name no type.
(attribute
  (user_type (type_identifier) @ignore))

; Calls: of a function or a type (`fee(x)`, `Order(size: 1)`, `run { }`, `handler!(x)`) and of a
; member (`order.pay()`, `item?.pay()`, `super.init()`). The grammar writes a subscript (`items[0]`)
; as a call whose arguments open with `[`, and `defer { }` as a call to `defer`: neither is one.
((call_expression
   [(simple_identifier) @name
    (postfix_expression target: (simple_identifier) @name)]
   (call_suffix [(lambda_literal) (value_arguments "(")]))
 (#not-eq? @name "defer")) @reference.call
(call_expression
  [(navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))
   (postfix_expression
     target: (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name)))]
  (call_suffix [(lambda_literal) (value_arguments "(")])) @reference.call

; The grammar gives a call's arguments to a whole sum, product or prefixed operand before it:
; `a + f(x)` reads as `(a + f)(x)`, `-f(x)` as `(-f)(x)` and `.tags(x)` as `(.tags)(x)`. The
; call is then the last operand's, two operators deep at most (`a + b * f(x)`).
(call_expression
  [(additive_expression
     rhs: [(simple_identifier) @name
           (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))
           (multiplicative_expression
             rhs: [(simple_identifier) @name
                   (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))])
           (prefix_expression
             target: [(simple_identifier) @name
                      (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))])])
   (multiplicative_expression
     rhs: [(simple_identifier) @name
           (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))
           (prefix_expression
             target: [(simple_identifier) @name
                      (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))])])
   (prefix_expression
     target: [(simple_identifier) @name
              (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))])]
  (call_suffix [(lambda_literal) (value_arguments "(")])) @reference.call

; Bases: the superclass and protocols of a type or an extension, plain, generic or qualified.
(inheritance_specifier
  inherits_from: [(user_type (type_identifier) @name .)
                  (user_type (type_identifier) @name . (type_arguments) .)]) @reference.inherit

; Every other use of a type: annotations, parameters, results, generic arguments and constraints,
; optionals, arrays, casts and checks.
(user_type (type_identifier) @name .) @reference.type
(user_type (type_identifier) @name . (type_arguments) .) @reference.type
