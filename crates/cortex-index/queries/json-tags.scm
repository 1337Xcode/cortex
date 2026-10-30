; Cortex's tags query for JSON; the grammar ships none.

; A pair the grammar could not read whole is not read: its key may be part of a broken one, as
; `b` is in `"b"c": 2`.
(pair
  key: (string . (string_content) @ignore .)
  (ERROR))

; The keys of the object a file holds, each a constant spanning its value, its header the key:
; `"name": "shop"`. A key holding an escape is not read, as the grammar splits its text.
(document
  (object
    (pair
      key: (string . (string_content) @name .)) @definition.constant))
