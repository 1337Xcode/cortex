; Additions to the tags query that ships with the Python grammar, compiled before it. Where two
; patterns find the same name the earlier one wins, so definitions come before references.

; Decorated functions and classes, read with their decorators. The grammar's own patterns would
; read the same names without them.
(decorated_definition
  definition: (function_definition name: (identifier) @name)) @definition.function
(decorated_definition
  definition: (class_definition name: (identifier) @name)) @definition.class

; Type aliases: `type Alias = ...` and `type Pair[T] = ...`.
(type_alias_statement
  left: (type
    [(identifier) @name
     (generic_type (identifier) @name)])) @definition.type

; Base classes, plain, qualified or generic: `class A(Base, module.Mixin, Generic[T])`. Keyword
; arguments such as `metaclass=` are not bases.
(class_definition
  superclasses: (argument_list
    [(identifier) @name
     (attribute attribute: (identifier) @name)
     (subscript value: (identifier) @name)
     (subscript value: (attribute attribute: (identifier) @name))] @reference.inherit))

; Decorators without arguments are applied as calls: `@property`, `@functools.cache`. Decorators
; with arguments are calls already.
(decorator
  [(identifier) @name
   (attribute attribute: (identifier) @name)] @reference.call)

; Names in annotations: plain or qualified, and the head of a generic such as `list[Item]`. The
; members of a union of generics are annotations themselves.
(type (identifier) @name @reference.type)
(type (attribute attribute: (identifier) @name) @reference.type)
(generic_type (identifier) @name @reference.type)

; A union of plain names, `A | B | None`, is parsed as `|` operators nested to the left. The names
; in unions of up to three members are read; a longer union loses its first members.
(type (binary_operator left: (identifier) @name @reference.type))
(type (binary_operator right: (identifier) @name @reference.type))
(type (binary_operator left: (binary_operator left: (identifier) @name @reference.type)))
(type (binary_operator left: (binary_operator right: (identifier) @name @reference.type)))
