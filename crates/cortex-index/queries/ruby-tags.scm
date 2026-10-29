; Cortex's patterns for Ruby, compiled before the tags query that ships with the Ruby grammar:
; where a pattern here and one of the grammar's find the same name, this one wins.

; Scopes the grammar's locals query leaves out. A method defined on an object
; (`def self.build(size)`) and the body of a class or a module start with no local variables,
; as a `def` does, so a parameter of such a method is local to it alone.
([(singleton_method) (class) (module) (singleton_class)] @local.scope
 (#set! local.scope-inherits false))

; A method or an alias named by a symbol or a variable, as `alias :total :sum` and
; `alias $stdout $stderr` name them: the name would carry its sigil, and an interpolated symbol
; holds code, so it is not recorded.
[
  (alias
    name: [(simple_symbol) (delimited_symbol) (global_variable) (instance_variable)
           (class_variable)] @ignore)
  (method
    name: [(simple_symbol) (delimited_symbol) (global_variable) (instance_variable)
           (class_variable)] @ignore)
  (singleton_method
    name: [(simple_symbol) (delimited_symbol) (global_variable) (instance_variable)
           (class_variable)] @ignore)
]

; Methods defined at the top of a file, outside any class or module: functions.
(program
  (method
    name: (_) @name) @definition.function)

; Constants: `LIMIT = 3`, `Point = Struct.new(:x, :y)`, `RATE ||= 2`. The grammar's query reads
; the name as a use of the constant.
[
  (assignment
    left: (constant) @name)
  (operator_assignment
    left: (constant) @name)
] @definition.constant

; Base classes, plain or qualified: `class Order < Base`, `class Report < ActiveRecord::Base`. The
; grammar's query reads them as uses of the constant.
(superclass
  [(constant) @name
   (scope_resolution
     name: (constant) @name)]) @reference.inherit

; Modules mixed into a class or module, plain or qualified, one or several: `include Comparable`,
; `extend Forwardable`, `prepend Audit::Logging`.
((call
   !receiver
   method: (identifier) @_mixin
   arguments: (argument_list
     [(constant) @name
      (scope_resolution
        name: (constant) @name)])) @reference.inherit
 (#any-of? @_mixin "include" "extend" "prepend"))
