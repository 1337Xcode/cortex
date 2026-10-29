; Components used in JSX, in JavaScript and TSX files: `<Widget />` and `<Layout.Header>`.
; Lower-case names are HTML elements.
((jsx_opening_element name: (identifier) @name @reference.call)
  (#match? @name "^[A-Z]"))
((jsx_self_closing_element name: (identifier) @name @reference.call)
  (#match? @name "^[A-Z]"))
(jsx_opening_element
  name: (member_expression property: (property_identifier) @name) @reference.call)
(jsx_self_closing_element
  name: (member_expression property: (property_identifier) @name) @reference.call)
