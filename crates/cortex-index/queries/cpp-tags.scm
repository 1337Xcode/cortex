; Cortex's tags query for C++. The query that ships with the C++ grammar reads no references, gives
; a function the span of its declarator alone, reads every prototype as a definition, and marks a
; method's class with a `@local.scope` capture Cortex reads as a scope, so it is not used. Names
; qualified by up to two scopes are read (`a::b::C`), and methods defined outside up to three
; classes or namespaces; deeper ones are not.

; Google Test's tests, `TEST(OrderTest, Pays) {...}`, which the grammar reads as a function named
; `TEST` with two parameters: the test is the second, inside the suite the first names. The macro
; is no function, and the suite of `TEST` and `TEST_P` is no type (that of `TEST_F` is a class).
((function_definition
   declarator: (function_declarator
     declarator: (identifier) @_macro
     parameters: (parameter_list
       .
       (parameter_declaration type: (type_identifier) @qualifier)
       .
       (parameter_declaration type: (type_identifier) @name)
       .))) @definition.test
 (#any-of? @_macro "TEST" "TEST_F" "TEST_P" "TYPED_TEST" "TYPED_TEST_P"))
((function_definition
   declarator: (function_declarator
     declarator: (identifier) @ignore))
 (#any-of? @ignore "TEST" "TEST_F" "TEST_P" "TYPED_TEST" "TYPED_TEST_P"))
((function_definition
   declarator: (function_declarator
     declarator: (identifier) @_macro
     parameters: (parameter_list
       .
       (parameter_declaration type: (type_identifier) @ignore))))
 (#any-of? @_macro "TEST" "TEST_P"))

; A friend function defined in a class is not a member of it, and is not read; neither is a
; deleted function, `Order(const Order&) = delete;`.
(friend_declaration
  (function_definition
    declarator: (function_declarator
      declarator: [(identifier) @ignore
                   (operator_name) @ignore
                   (template_function name: (identifier) @ignore)])))
(function_definition
  declarator: (function_declarator
    declarator: [(identifier) @ignore
                 (field_identifier) @ignore
                 (operator_name) @ignore
                 (destructor_name) @ignore])
  (delete_method_clause))

; Functions, and those returning a pointer, a pointer to a pointer or a reference, named plainly,
; as an operator (`operator==`), a destructor (`~Order`) or a specialization (`max<int>`). One
; defined inside a class is a method. A prototype, a method declared without its body and a
; conversion operator (`operator bool`) are not definitions.
(function_definition
  declarator: [(function_declarator
                 declarator: [(identifier) @name
                              (field_identifier) @name
                              (operator_name) @name
                              (destructor_name) @name
                              (template_function name: (identifier) @name)])
               (pointer_declarator
                 declarator: (function_declarator
                   declarator: [(identifier) @name
                                (field_identifier) @name
                                (operator_name) @name
                                (template_function name: (identifier) @name)]))
               (pointer_declarator
                 declarator: (pointer_declarator
                   declarator: (function_declarator
                     declarator: [(identifier) @name
                                  (field_identifier) @name
                                  (operator_name) @name
                                  (template_function name: (identifier) @name)])))
               (reference_declarator
                 (function_declarator
                   declarator: [(identifier) @name
                                (field_identifier) @name
                                (operator_name) @name
                                (template_function name: (identifier) @name)]))]) @definition.function

; Methods defined outside their class, `int Order::total() const {...}`, `Stack<T>::pop()`,
; `shop::Order::total()`: the innermost class goes before the method's name.
(function_definition
  declarator: [(function_declarator
                 declarator: [(qualified_identifier
                               scope: [(namespace_identifier) @qualifier
                                       (template_type name: (type_identifier) @qualifier)]
                               name: [(identifier) @name
                                      (operator_name) @name
                                      (destructor_name) @name
                                      (template_function name: (identifier) @name)])
                              (qualified_identifier
                                name: (qualified_identifier
                                  scope: [(namespace_identifier) @qualifier
                                          (template_type name: (type_identifier) @qualifier)]
                                  name: [(identifier) @name
                                         (operator_name) @name
                                         (destructor_name) @name
                                         (template_function name: (identifier) @name)]))
                              (qualified_identifier
                                name: (qualified_identifier
                                  name: (qualified_identifier
                                    scope: [(namespace_identifier) @qualifier
                                            (template_type name: (type_identifier) @qualifier)]
                                    name: [(identifier) @name
                                           (operator_name) @name
                                           (destructor_name) @name
                                           (template_function name: (identifier) @name)])))])
               (pointer_declarator
                 declarator: (function_declarator
                   declarator: (qualified_identifier
                     scope: [(namespace_identifier) @qualifier
                             (template_type name: (type_identifier) @qualifier)]
                     name: [(identifier) @name (operator_name) @name])))
               (reference_declarator
                 (function_declarator
                   declarator: (qualified_identifier
                     scope: [(namespace_identifier) @qualifier
                             (template_type name: (type_identifier) @qualifier)]
                     name: [(identifier) @name (operator_name) @name])))]) @definition.method

; Classes, structs and unions with a body are classes, named plainly, as a specialization
; (`template <> class Hash<Order> {...}`) or with a scope (`struct std::hash<Order> {...}`);
; enums with a body (`enum class` too) are types. Without a body, as in `class Order;`, the name
; is a use of the type, read at the end.
(class_specifier
  name: [(type_identifier) @name
         (template_type name: (type_identifier) @name)
         (qualified_identifier name: (type_identifier) @name)
         (qualified_identifier name: (template_type name: (type_identifier) @name))]
  body: (_)) @definition.class
(struct_specifier
  name: [(type_identifier) @name
         (template_type name: (type_identifier) @name)
         (qualified_identifier name: (type_identifier) @name)
         (qualified_identifier name: (template_type name: (type_identifier) @name))]
  body: (_)) @definition.class
(union_specifier
  name: [(type_identifier) @name
         (template_type name: (type_identifier) @name)]
  body: (_)) @definition.class
(enum_specifier
  name: (type_identifier) @name
  body: (_)) @definition.type

; A class whose name follows an export macro, `class API Order {...}`, which the grammar reads as
; a function `Order` returning `class API`: the class is `Order`, its members are methods, and the
; macro is no type.
(function_definition
  type: [(class_specifier name: (type_identifier) @ignore !body)
         (struct_specifier name: (type_identifier) @ignore !body)]
  declarator: (identifier)
  body: (_))
(function_definition
  type: [(class_specifier !body) (struct_specifier !body)]
  declarator: (identifier) @name
  body: (_)) @definition.class

; Namespaces are modules; `namespace shop::util {...}` is named by its last part.
(namespace_definition
  name: [(namespace_identifier) @name
         (nested_namespace_specifier
           (namespace_identifier) @name
           .)
         (nested_namespace_specifier
           (nested_namespace_specifier
             (namespace_identifier) @name
             .)
           .)
         (nested_namespace_specifier
           (nested_namespace_specifier
             (nested_namespace_specifier
               (namespace_identifier) @name
               .)
             .)
           .)]) @definition.module

; Type names: `using money = long;`, and those `typedef` gives, read as in C. When the typedef
; also defines a named class, struct, union or enum, each name it gives spans itself alone.
(alias_declaration
  name: (type_identifier) @name) @definition.type
(type_definition
  type: [(class_specifier name: (_) body: (_))
         (struct_specifier name: (_) body: (_))
         (union_specifier name: (_) body: (_))
         (enum_specifier name: (_) body: (_))]
  declarator: [(type_identifier) @name @definition.type
               (pointer_declarator declarator: (type_identifier) @name @definition.type)
               (array_declarator declarator: (type_identifier) @name @definition.type)])
(type_definition
  type: (_)
  .
  declarator: [(type_identifier) @name
               (pointer_declarator declarator: (type_identifier) @name)
               (array_declarator declarator: (type_identifier) @name)
               (function_declarator
                 declarator: (parenthesized_declarator
                   (pointer_declarator declarator: (type_identifier) @name)))]
  .) @definition.type
(type_definition
  declarator: [(type_identifier) @name @definition.type
               (pointer_declarator declarator: (type_identifier) @name @definition.type)
               (array_declarator declarator: (type_identifier) @name @definition.type)
               (function_declarator
                 declarator: (parenthesized_declarator
                   (pointer_declarator declarator: (type_identifier) @name @definition.type)))])

; Macros, as in C: `#define LIMIT 3` is a constant, `#define MAX(a, b) ...` a macro, an include
; guard nothing.
(preproc_def
  name: (identifier) @name
  value: (_)) @definition.constant
(preproc_function_def
  name: (identifier) @name) @definition.macro

; Variables declared in a block, as in a function's body, or in a `for`, are not constants.
(compound_statement
  (declaration
    declarator: (init_declarator
      declarator: [(identifier) @ignore
                   (array_declarator declarator: (identifier) @ignore)
                   (pointer_declarator declarator: (identifier) @ignore)
                   (pointer_declarator
                     declarator: (array_declarator declarator: (identifier) @ignore))])))
(for_statement
  initializer: (declaration
    declarator: (init_declarator
      declarator: [(identifier) @ignore
                   (array_declarator declarator: (identifier) @ignore)
                   (pointer_declarator declarator: (identifier) @ignore)
                   (pointer_declarator
                     declarator: (array_declarator declarator: (identifier) @ignore))])))

; Constants: variables that cannot change, `const` or `constexpr`, anywhere else (a namespace, the
; top of the file), as in C; and a class's `const` or `constexpr` member with a value, alone in its
; declaration.
((declaration
   (type_qualifier) @_const
   declarator: (init_declarator
     declarator: [(identifier) @name
                  (array_declarator declarator: (identifier) @name)]
     value: (_)) @definition.constant)
 (#any-of? @_const "const" "constexpr"))
((declaration
   declarator: (init_declarator
     declarator: (pointer_declarator
       (type_qualifier) @_const
       declarator: [(identifier) @name
                    (array_declarator declarator: (identifier) @name)])
     value: (_)) @definition.constant)
 (#eq? @_const "const"))
((field_declaration
   (type_qualifier) @_const
   .
   type: (_)
   .
   declarator: (field_identifier) @name
   .
   default_value: (_)
   .) @definition.constant
 (#any-of? @_const "const" "constexpr"))

; The arguments of an attribute are not calls.
(attribute_specifier
  (argument_list
    (call_expression
      function: (identifier) @ignore)))

; Calls: of a function or a macro, through an object (`o.pay()`, `p->pay()`), qualified
; (`std::max(a, b)`, `shop::util::log(x)`), with template arguments (`make_unique<Order>()`).
; A cast (`static_cast<int>(x)`) is not a call.
((call_expression
   function: [(identifier) @name
              (field_expression field: (field_identifier) @name)
              (field_expression
                field: (template_method name: (field_identifier) @name))
              (template_function name: (identifier) @name)
              (qualified_identifier
                name: [(identifier) @name
                       (template_function name: (identifier) @name)])
              (qualified_identifier
                name: (qualified_identifier
                  name: [(identifier) @name
                         (template_function name: (identifier) @name)]))]) @reference.call
 (#not-any-of? @name "static_cast" "dynamic_cast" "const_cast" "reinterpret_cast"))

; Creating an object calls its constructor: `new Order()`, `new shop::Order()`.
(new_expression
  type: [(type_identifier) @name
         (template_type name: (type_identifier) @name)
         (qualified_identifier
           name: [(type_identifier) @name
                  (template_type name: (type_identifier) @name)])
         (qualified_identifier
           name: (qualified_identifier
             name: [(type_identifier) @name
                    (template_type name: (type_identifier) @name)]))]) @reference.call

; Base classes, plain, qualified or with template arguments: `class Stack : public Base<T>`.
(base_class_clause
  [(type_identifier) @name
   (template_type name: (type_identifier) @name)
   (qualified_identifier
     name: [(type_identifier) @name
            (template_type name: (type_identifier) @name)])
   (qualified_identifier
     name: (qualified_identifier
       name: [(type_identifier) @name
              (template_type name: (type_identifier) @name)]))]) @reference.inherit

; Uses of a type: in a `type:` field (a declaration, a parameter, a field, a cast, a template
; argument, a function's result), plain, qualified or with template arguments; and a class,
; struct, union or enum named without a body.
(_
  type: [(type_identifier) @name
         (template_type name: (type_identifier) @name)
         (qualified_identifier
           name: [(type_identifier) @name
                  (template_type name: (type_identifier) @name)])
         (qualified_identifier
           name: (qualified_identifier
             name: [(type_identifier) @name
                    (template_type name: (type_identifier) @name)]))] @reference.type)
(class_specifier
  name: (type_identifier) @name @reference.type
  !body)
(struct_specifier
  name: (type_identifier) @name @reference.type
  !body)
(union_specifier
  name: (type_identifier) @name @reference.type
  !body)
(enum_specifier
  name: (type_identifier) @name @reference.type
  !body)
