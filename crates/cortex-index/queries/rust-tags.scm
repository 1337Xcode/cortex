; Cortex's patterns for Rust, compiled before the tags query that ships with the Rust grammar:
; where a pattern here and one of the grammar's find the same name, this one wins.

; `Self` names the type an `impl` is for, never a definition of its own.
((type_identifier) @ignore
  (#eq? @ignore "Self"))

; Functions marked `#[test]`, or with a test attribute from a crate such as `#[tokio::test]`, other
; attributes between allowed: tests.
((attribute_item
   (attribute
     [(identifier) @_attribute
      (scoped_identifier name: (identifier) @_attribute)]))
 .
 (attribute_item)*
 .
 (function_item name: (identifier) @name) @definition.test
 (#eq? @_attribute "test"))

; Methods and associated constants in `impl` blocks, named with the type: `Order::new` is
; `Order.new`, in an inherent impl or a trait's.
(impl_item
  type: [(type_identifier) @qualifier
         (generic_type type: (type_identifier) @qualifier)
         (scoped_type_identifier name: (type_identifier) @qualifier)]
  body: (declaration_list
    (function_item name: (identifier) @name) @definition.method))
(impl_item
  type: [(type_identifier) @qualifier
         (generic_type type: (type_identifier) @qualifier)
         (scoped_type_identifier name: (type_identifier) @qualifier)]
  body: (declaration_list
    (const_item name: (identifier) @name) @definition.constant))

; Functions in a module's body are functions; the grammar's query reads every function in a
; declaration list as a method.
(mod_item
  body: (declaration_list
    (function_item name: (identifier) @name) @definition.function))

; Methods a trait declares without a body: `fn price(&self) -> Money;`.
(function_signature_item name: (identifier) @name) @definition.method

; Type aliases are types, not classes as the grammar's query reads them.
(type_item name: (type_identifier) @name) @definition.type

; Constants and statics: `const MAX: usize = 50;`, `static NAME: &str = "x";`.
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant

; An inherent `impl Order` uses the type; the grammar's query reads it as an implementation, as it
; does a trait's.
(impl_item
  type: [(type_identifier) @name
         (generic_type type: (type_identifier) @name)]
  !trait) @reference.type

; Calls through a path: `Order::new()`, `Vec::with_capacity(4)`.
(call_expression
  function: (scoped_identifier name: (identifier) @name)) @reference.call
