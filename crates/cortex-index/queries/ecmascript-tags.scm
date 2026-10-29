; Additions shared by JavaScript and TypeScript, compiled after the grammars' own tags queries.

; Constants declared at the top of a module, exported or not: `const LIMIT = 10`. One holding a
; function is a function, as the grammars' own patterns read it.
(program
  (lexical_declaration
    kind: "const"
    (variable_declarator name: (identifier) @name) @definition.constant))
(program
  (export_statement
    declaration: (lexical_declaration
      kind: "const"
      (variable_declarator name: (identifier) @name) @definition.constant)))
