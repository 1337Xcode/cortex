; Cortex's tags query for SQL; the grammar ships none.

; A statement holding a syntax error among its own parts, after its name or before it, is not
; read: the grammar could not read it whole, so its name may be cut short (`a1` of `a1$`) or a
; keyword (`IF` of a function's `IF NOT EXISTS`), or it may have run into the next statement or
; the one before into it (`CREATE VIRTUAL TABLE`).
(statement
  (_ (object_reference name: (identifier) @ignore) (ERROR)))
(statement
  (_ (ERROR) (object_reference name: (identifier) @ignore)))

; A name with a database before its schema is not read: a qualified name holds one qualifier, and
; the name without its database would be part of it.
(object_reference
  database: (identifier)
  name: (identifier) @ignore)

; What a script creates at its top, or in a transaction there, not in a function's body: tables,
; views and types, each a type, and functions, each a function. A schema before the name qualifies
; it (`public.orders`). Names are read as written: a quoted name keeps its quotes, as dumps quote
; every name, and the grammar makes tokens of the backticks of a MySQL name and none of the name
; between them. The header runs up to the columns, the query, the values or the body. A statement
; that creates a table, a view or a type names no other object beside its own: the tables it uses
; sit inside its columns or its query.
(program
  (statement
    [(create_table
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       [(column_definitions) (create_query)]? @body)
     (create_view
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       (create_query) @body)
     (create_materialized_view
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       (create_query) @body)
     (create_type
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       [(enum_elements) (column_definitions)]? @body)] @definition.type))
(transaction
  (statement
    [(create_table
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       [(column_definitions) (create_query)]? @body)
     (create_view
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       (create_query) @body)
     (create_materialized_view
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       (create_query) @body)
     (create_type
       (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
       [(enum_elements) (column_definitions)]? @body)] @definition.type))

; A function's name is the object right after `FUNCTION`: the type it returns can be an object
; beside it.
(program
  (statement
    (create_function
      (keyword_function)
      .
      (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
      (function_body) @body) @definition.function))
(transaction
  (statement
    (create_function
      (keyword_function)
      .
      (object_reference schema: (identifier)? @qualifier name: (identifier) @name.prose)
      (function_body) @body) @definition.function))
