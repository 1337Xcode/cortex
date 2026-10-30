; Cortex's tags query for YAML; the grammar ships none.

; The keys of the mapping each document of a file holds, each a constant spanning its value, its
; header the key: `name: build`. A key is read as written, a quoted one with its quotes; a key that
; is a number, a boolean or a structure names nothing and is not read, and nor does the merge key
; `<<`, which merges another mapping into this one.
((document
   (block_node
     (block_mapping
       (block_mapping_pair
         key: (flow_node
           [(plain_scalar (string_scalar))
            (double_quote_scalar)
            (single_quote_scalar)] @name.prose)) @definition.constant)))
 (#not-eq? @name.prose "<<"))
