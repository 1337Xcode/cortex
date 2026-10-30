; Cortex's tags query for Bash and the shells close to it; the grammar ships none.

; Functions, declared in any of the three ways a script can: `deploy() { }`, `function deploy() { }`
; and `function deploy { }`.
(function_definition
  name: (word) @name) @definition.function

; Constants: a variable declared read-only at the top of a script, `readonly RATE=2` or
; `declare -r LIMIT=3`, each assignment of the declaration its own. Inside a function, `declare`
; and `typeset` make a local variable, and `readonly` a global one the function's name would
; wrongly qualify, so neither is read there.
(program
  (declaration_command
    "readonly"
    (variable_assignment
      name: (variable_name) @name) @definition.constant))
((program
   (declaration_command
     ["declare" "typeset"]
     (word) @_flag
     (variable_assignment
       name: (variable_name) @name) @definition.constant))
 (#match? @_flag "^-[a-zA-Z]*r"))

; A command holding a syntax error is no call, and nor is a command in one of its parts after the
; error: the grammar has misread a construct it does not know, as a function whose body is a loop
; (`f() while read x; do :; done`) or zsh's `foreach x (a b c)`.
(command
  name: (command_name (word) @ignore)
  (ERROR))
(command
  (ERROR)
  (_
    (command
      name: (command_name (word) @ignore))))

; Calls: every command a script runs by its name or its path, a function of its own or a program;
; an include (`source`, `.`) is an import instead. Bash's reserved words, and those zsh adds for
; its loops and for `nocorrect`, name no command, so one in a command's place is the grammar
; misreading a construct, as the `done` of a loop it could not read.
((command
   name: (command_name (word) @name)) @reference.call
 (#not-any-of? @name
   "source" "."
   "!" "[[" "]]" "{" "}" "case" "coproc" "do" "done" "elif" "else" "esac" "fi" "for" "function"
   "if" "in" "select" "then" "time" "until" "while"
   "end" "foreach" "nocorrect" "repeat"))
