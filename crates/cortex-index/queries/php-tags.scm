; Cortex's tags query for PHP. The query that ships with the PHP grammar reads properties with a
; kind outside the tags format (`@definition.field`), so it is not used.

; PHPUnit's tests marked by the `Test` attribute, `#[Test]` or
; `#[\PHPUnit\Framework\Attributes\Test]`, or by an `@test` line in the comment just before
; them, anywhere; methods named `test...` are tests in test files.
((method_declaration
   attributes: (attribute_list
     (attribute_group
       (attribute
         [(name) @_attribute
          (qualified_name
            prefix: (namespace_name) @_library
            (name) @_attribute
            .)])))
   name: (name) @name) @definition.test
 (#eq? @_attribute "Test")
 (#match? @_library "PHPUnit"))
((comment) @_doc
 .
 (method_declaration
   name: (name) @name) @definition.test
 (#match? @_doc "@test\\b"))

; Declarations after a namespace written as a statement, `namespace App\Orders;`, which the grammar
; does not place inside it: the namespace goes before their names, as a block namespace's does by
; holding them. After a second such statement, the nearer namespace is theirs. A declaration
; inside a statement, as a function defined under an `if`, takes none.
(program
  (namespace_definition
    name: (_) @qualifier)
  [(class_declaration name: (name) @name)
   (enum_declaration name: (name) @name)] @definition.class)
(program
  (namespace_definition
    name: (_) @qualifier)
  [(interface_declaration name: (name) @name)
   (trait_declaration name: (name) @name)] @definition.interface)
(program
  (namespace_definition
    name: (_) @qualifier)
  (function_definition
    name: (name) @name) @definition.function)
(program
  (namespace_definition
    name: (_) @qualifier)
  (const_declaration
    (const_element
      .
      (name) @name) @definition.constant))

; Classes and enums are classes; interfaces and traits interfaces; namespaces modules, named as
; written (`App\Orders`).
[(class_declaration name: (name) @name)
 (enum_declaration name: (name) @name)] @definition.class
[(interface_declaration name: (name) @name)
 (trait_declaration name: (name) @name)] @definition.interface
(namespace_definition
  name: (_) @name) @definition.module

; Functions and methods; a method declared without a body, in an interface or as abstract, is a
; method too.
(function_definition
  name: (name) @name) @definition.function
(method_declaration
  name: (name) @name) @definition.method

; Constants: `const LIMIT = 3;` in a class or at the top of a file, each spanning its element, and
; `define('MAX', 9)` in any letter case, named by its string. Enum cases are not read.
(const_element
  .
  (name) @name) @definition.constant
((function_call_expression
   function: (name) @_define
   arguments: (arguments
     .
     (argument
       [(string
          .
          (string_content) @name
          .)
        (encapsed_string
          .
          (string_content) @name
          .)]))) @definition.constant
 (#match? @_define "^[dD][eE][fF][iI][nN][eE]$"))

; Calls: of a function, plain or qualified (`format($x)`, `\App\log($x)`), of a method on an object
; (`$order->pay()`, `$order?->pay()`) or a class (`Helper::round($x)`, `parent::__construct()`).
(function_call_expression
  function: [(name) @name
             (qualified_name (name) @name .)]) @reference.call
[(member_call_expression name: (name) @name)
 (nullsafe_member_call_expression name: (name) @name)
 (scoped_call_expression name: (name) @name)] @reference.call

; Creating an object calls its constructor: `new Order()`, `new \App\Models\Order()`; `new self`,
; `new static` and `new parent` name no class.
((object_creation_expression
   [(name) @name
    (qualified_name (name) @name .)]) @reference.call
 (#not-any-of? @name "self" "static" "parent"))

; Base classes, interfaces implemented or extended, and traits used in a class.
(base_clause
  [(name) @name
   (qualified_name (name) @name .)]) @reference.inherit
(class_interface_clause
  [(name) @name
   (qualified_name (name) @name .)]) @reference.inherit
(use_declaration
  [(name) @name
   (qualified_name (name) @name .)]) @reference.inherit

; Uses of a type: a parameter's, a property's or a function's result type, nullable or in a union,
; a caught exception, and the class after `instanceof`; `self`, `static` and `parent` name none.
((named_type
   [(name) @name
    (qualified_name (name) @name .)]) @reference.type
 (#not-any-of? @name "self" "static" "parent"))
(binary_expression
  "instanceof"
  right: [(name) @name
          (qualified_name (name) @name .)]) @reference.type
