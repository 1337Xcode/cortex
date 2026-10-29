; Cortex's tags query for C#. The query that ships with the C# grammar uses a capture outside the
; tags format (`@module`), and reads base types as uses and calls with a kind of its own, so it is
; not used.

; Methods marked as tests by NUnit, xUnit or MSTest: `[Test]`, `[TestCase(1)]`, `[Fact]`,
; `[Theory]`, `[TestMethod]`, or the same with the `Attribute` suffix. A qualified attribute is
; not read, since its last part alone could belong to any library (`[Moq.Test]`).
((method_declaration
   (attribute_list
     (attribute
       name: (identifier) @_attribute))
   name: (identifier) @name) @definition.test
 (#any-of? @_attribute "Test" "TestCase" "TestCaseSource" "Fact" "Theory" "TestMethod"
   "DataTestMethod" "TestAttribute" "TestCaseAttribute" "TestCaseSourceAttribute"
   "FactAttribute" "TheoryAttribute" "TestMethodAttribute" "DataTestMethodAttribute"))

; Types declared at the top of a file that has a file-scoped namespace, `namespace Shop.Orders;`:
; the namespace goes before the type's name, as a block namespace's does by holding the type.
(compilation_unit
  (file_scoped_namespace_declaration
    name: (_) @qualifier)
  [(class_declaration name: (identifier) @name)
   (struct_declaration name: (identifier) @name)
   (record_declaration name: (identifier) @name)
   (enum_declaration name: (identifier) @name)] @definition.class)
(compilation_unit
  (file_scoped_namespace_declaration
    name: (_) @qualifier)
  (interface_declaration
    name: (identifier) @name) @definition.interface)
(compilation_unit
  (file_scoped_namespace_declaration
    name: (_) @qualifier)
  (delegate_declaration
    name: (identifier) @name) @definition.type)

; Classes, structs, records and enums are classes, interfaces interfaces, delegates types.
[(class_declaration name: (identifier) @name)
 (struct_declaration name: (identifier) @name)
 (record_declaration name: (identifier) @name)
 (enum_declaration name: (identifier) @name)] @definition.class
(interface_declaration
  name: (identifier) @name) @definition.interface
(delegate_declaration
  name: (identifier) @name) @definition.type

; Namespaces are modules, named as written: `namespace Shop.Orders` is `Shop.Orders`.
[(namespace_declaration name: (_) @name)
 (file_scoped_namespace_declaration name: (_) @name)] @definition.module

; Methods and constructors (named after their class). An interface's or an abstract method,
; without a body, is a method too. Properties, indexers, events, operators and finalizers are
; not read, and neither are functions declared inside a member, which could otherwise sit
; directly in a class when their member is not read.
[(method_declaration name: (identifier) @name)
 (constructor_declaration name: (identifier) @name)] @definition.method

; Constants: `const` fields, and `static readonly` ones in either order. Each spans its declarator,
; `Limit = 3`.
((field_declaration
   (modifier) @_const
   (variable_declaration
     (variable_declarator
       name: (identifier) @name) @definition.constant))
 (#eq? @_const "const"))
((field_declaration
   (modifier) @_first
   (modifier) @_second
   (variable_declaration
     (variable_declarator
       name: (identifier) @name) @definition.constant))
 (#any-of? @_first "static" "readonly")
 (#any-of? @_second "static" "readonly"))

; Calls: plain, through an object or a type (`items.Add(x)`, `Console.WriteLine(x)`), when the
; object is not null (`order?.Pay()`), with type arguments (`Helper.Format<Order>(x)`).
; `nameof(x)` names `x` and calls nothing.
((invocation_expression
   function: [(identifier) @name
              (generic_name (identifier) @name)
              (member_access_expression
                name: [(identifier) @name
                       (generic_name (identifier) @name)])
              (conditional_access_expression
                (member_binding_expression
                  name: [(identifier) @name
                         (generic_name (identifier) @name)]))]) @reference.call
 (#not-eq? @name "nameof"))

; Creating an object calls its constructor: `new Order()`, `new List<Item>()`,
; `new System.Text.StringBuilder()`.
(object_creation_expression
  type: [(identifier) @name
         (generic_name (identifier) @name)
         (qualified_name
           name: [(identifier) @name
                  (generic_name (identifier) @name)])]) @reference.call

; Base types, classes or interfaces: `class Order : Base<Item>, IPriced`, and a record's base
; given its arguments, `record Paid(int Id) : Event(Id)`.
(base_list
  [(identifier) @name
   (generic_name (identifier) @name)
   (qualified_name
     name: [(identifier) @name
            (generic_name (identifier) @name)])
   (primary_constructor_base_type
     type: [(identifier) @name
            (generic_name (identifier) @name)
            (qualified_name
              name: [(identifier) @name
                     (generic_name (identifier) @name)])])]) @reference.inherit

; Uses of a type: in a `type:` or `returns:` field (a declaration, a parameter, a property, a
; cast, a method's result), nullable or an array, plain, generic or qualified; after `as`; and a
; type argument, as `Item` in `List<Item>`. After `is`, the grammar cannot tell a type from a
; constant, so that name is not read.
(_
  type: [(identifier) @name
         (generic_name (identifier) @name)
         (qualified_name
           name: [(identifier) @name
                  (generic_name (identifier) @name)])] @reference.type)
(_
  returns: [(identifier) @name
            (generic_name (identifier) @name)
            (qualified_name
              name: [(identifier) @name
                     (generic_name (identifier) @name)])] @reference.type)
(as_expression
  right: [(identifier) @name
          (generic_name (identifier) @name)
          (qualified_name
            name: [(identifier) @name
                   (generic_name (identifier) @name)])] @reference.type)
(type_argument_list
  [(identifier) @name
   (generic_name (identifier) @name)
   (qualified_name
     name: [(identifier) @name
            (generic_name (identifier) @name)])] @reference.type)
