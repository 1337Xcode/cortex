; Cortex's tags query for JSON; the grammar ships none.

; The keys of the object a file holds, each a constant spanning its value, its header the key:
; `"name": "shop"`. A key holding an escape is not read, as the grammar splits its text.
(document
  (object
    (pair
      key: (string . (string_content) @name .)) @definition.constant))
