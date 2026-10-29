# cortex-store

Storage for one repository: a SQLite database at `.cortex/cortex.db`, its numbered migrations, the
one writer, and read-only readers.

## What it must never do

- Store code, prompt text or tool output. The records the writer accepts have fields only for
  repository paths, names of one line (at most 256 bytes) and the fixed names of kinds. The store
  does not check that a name is not a line of code; that is the job of the code that builds the
  records.
- Let two writers open the same database. `Writer::open` takes an exclusive lock on
  `.cortex/writer.lock`, which the operating system releases when the writer closes or its process
  ends.
- Edit a published migration, build SQL by formatting values into it, or use a connection whose
  settings do not read back as required.

## Entry points

| Item | What it is |
| --- | --- |
| `Writer::open` | Takes the lock, creates `.cortex/` when it is missing and opens the database. A database Cortex did not write, or one a newer Cortex wrote, is refused untouched. When the database has an older schema, it is copied to `.cortex/backups/cortex-v<version>.db` (one copy per version, flushed to disk) and then migrated in one transaction |
| `Writer::transaction`, `WriteTx` | One transaction: `replace_file`, `remove_file`, `set_resolutions` and `replace_edges`, then `commit`. A write that fails is undone on its own, and the transaction keeps the writes before it. If SQLite rolls the whole transaction back after an error such as a full disk, every later write and the commit are refused |
| `Reader::open` | A read-only connection that can be open beside the writer |
| `Queries` | Reads shared by both: meta values, files, symbols, references, nodes, edges and row counts |
| `FileIndex` and the other records | What the writer accepts and reports. A symbol keeps its id, its graph node and the node's edges across edits while its qualified name and kind stay the same. Row ids are never reused, so an id kept from an earlier read names its old row or nothing. A reference can only point at an existing symbol and an edge can only join existing nodes; an edge goes with either of its nodes |

## Settings and files

Every connection uses write-ahead logging, `synchronous=NORMAL`, foreign keys, a 2-second busy
timeout, up to `store.mmap_bytes` of memory mapping (256 MiB by default) and in-memory temporary
tables. Each setting is read back after it is set.

On Unix, the store's folders and files are readable by their owner only (modes 0700 and 0600),
including ones it finds with a wider mode. On Windows they inherit the access list of the folder
they are created in.

## Tests

```sh
cargo test -p cortex-store
```

Expected values come from the table definitions typed into the schema test and from records built
by hand. The no-code test writes the records of a small source file and then searches the database
file's bytes for every line of that source.
