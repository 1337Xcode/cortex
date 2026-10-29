; Cortex's tags query for C. The query that ships with the C grammar reads no references, gives a
; function the span of its declarator alone, and reads a prototype as a definition, so it is not
; used.

; Functions, and those returning a pointer or a pointer to a pointer. A prototype is not a
; definition, and neither is a function returning a function pointer.
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @name))) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (pointer_declarator
      declarator: (function_declarator
        declarator: (identifier) @name)))) @definition.function

; Structs and unions with a body are classes, enums with one types. Without a body, as in
; `struct order *o`, the name is a use of the type, read at the end.
(struct_specifier
  name: (type_identifier) @name
  body: (_)) @definition.class
(union_specifier
  name: (type_identifier) @name
  body: (_)) @definition.class
(enum_specifier
  name: (type_identifier) @name
  body: (_)) @definition.type

; Type names given by `typedef`. When the typedef also defines a named struct, union or enum, as
; `typedef struct order {...} order_t;` does, each name it gives spans itself alone, so the struct
; does not sit inside it.
(type_definition
  type: [(struct_specifier name: (_) body: (_))
         (union_specifier name: (_) body: (_))
         (enum_specifier name: (_) body: (_))]
  declarator: [(type_identifier) @name @definition.type
               (pointer_declarator declarator: (type_identifier) @name @definition.type)
               (array_declarator declarator: (type_identifier) @name @definition.type)])

; A typedef that gives one name, right after its type, spans the whole typedef: `typedef int
; money;`, `typedef struct { int x; } point;`, `typedef int *intptr;`, `typedef char label[32];`,
; `typedef void (*pay_fn)(int);`.
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

; One that gives several names, `typedef int a, b;`, or has attributes after the name: each name
; spans itself alone, so none sits inside another. A name deeper in its declarator, as in
; `typedef int **grid;`, is not read.
(type_definition
  declarator: [(type_identifier) @name @definition.type
               (pointer_declarator declarator: (type_identifier) @name @definition.type)
               (array_declarator declarator: (type_identifier) @name @definition.type)
               (function_declarator
                 declarator: (parenthesized_declarator
                   (pointer_declarator declarator: (type_identifier) @name @definition.type)))])

; Macros: `#define LIMIT 3` is a constant and `#define MAX(a, b) ...` a macro, used as a function
; is. A name defined with no value, as an include guard is, is not read.
(preproc_def
  name: (identifier) @name
  value: (_)) @definition.constant
(preproc_function_def
  name: (identifier) @name) @definition.macro

; Variables declared in a block, as in a function's body, or in a `for`, are not constants,
; `const` or not.
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

; Constants: variables that cannot change, anywhere else (the top of the file, an `#if` block, an
; `extern "C"` block). A plain variable or an array is `const` itself, `const int RATE = 2`; a
; pointer has `const` after its star, `char *const NAME` or `const char *const NAMES[]`. A pointer
; to constant data, `const char *NAME`, is not a constant.
((declaration
   (type_qualifier) @_const
   declarator: (init_declarator
     declarator: [(identifier) @name
                  (array_declarator declarator: (identifier) @name)]
     value: (_)) @definition.constant)
 (#eq? @_const "const"))
((declaration
   declarator: (init_declarator
     declarator: (pointer_declarator
       (type_qualifier) @_const
       declarator: [(identifier) @name
                    (array_declarator declarator: (identifier) @name)])
     value: (_)) @definition.constant)
 (#eq? @_const "const"))

; The arguments of an attribute, as in `__attribute__((aligned(8)))`, are not calls.
(attribute_specifier
  (argument_list
    (call_expression
      function: (identifier) @ignore)))

; Calls, of a function or a macro, and through a struct field: `total(o)`, `MAX(1, 2)`,
; `o->pay(o)`.
(call_expression
  function: (identifier) @name) @reference.call
(call_expression
  function: (field_expression
    field: (field_identifier) @name)) @reference.call

; Uses of a type: the type of a declaration, a parameter, a field, a cast, `sizeof` or a function's
; result, and a struct, union or enum named without a body. A name a declarator gives is not one.
(_
  type: (type_identifier) @name @reference.type)
(struct_specifier
  name: (type_identifier) @name @reference.type
  !body)
(union_specifier
  name: (type_identifier) @name @reference.type
  !body)
(enum_specifier
  name: (type_identifier) @name @reference.type
  !body)
