# cortex-store

Storage for one repository: a SQLite database at `.cortex/cortex.db`, its numbered migrations, the
one writer, and read-only readers.

## What it must never do

- Store code, prompt text or tool output. The records the writer accepts have fields only for
  repository paths, names of one line (at most 256 bytes), the fixed names of kinds, numbers, a
  session's working folder and transcript path, and a prompt's hash, with its first 200
  characters only when the user asks for them. The store does not check that a name is not a line
  of code; that is the job of the code that builds the records.
- Let two writers open the same database. `Writer::open` takes an exclusive lock on
  `.cortex/writer.lock`, which the operating system releases when the writer closes or its process
  ends.
- Edit a published migration, build SQL by formatting values into it, or use a connection whose
  settings do not read back as required.

## Entry points

| Item | What it is |
| --- | --- |
| `Writer::open` | Takes the lock, creates `.cortex/` when it is missing and opens the database. A database Cortex did not write, or one a newer Cortex wrote, is refused untouched. When the database has an older schema, it is copied to `.cortex/backups/cortex-v<version>.db` (one copy per version, flushed to disk) and then migrated in one transaction |
| `Writer::transaction`, `WriteTx` | One transaction: `replace_file`, `remove_file`, `set_resolutions`, `replace_edges`, `folder_node` (a folder's node, added on first use and kept until removed), `remove_folder_node`, `set_scale_mode`, for co-change `clear_cochange`, `add_cochange` (counts summed with the stored ones, a pair's newest shared commit kept), `set_git_state`, `remove_git_state` and `remove_edges_of_type`, for what is learned `replace_interest`, and for agents' activity `record_session` (added, or updated with the facts given while keeping its first start time, and live again), `end_session`, `start_task` (numbered one past every task in the repository), `end_task` (once) and `append_event`, then `commit`. A write that fails is undone on its own, and the transaction keeps the writes before it. If SQLite rolls the whole transaction back after an error such as a full disk, every later write and the commit are refused |
| `Pause`, `Pacer`, `PAUSE_EVERY`, `wait_for` | How a long write (an index, a mining) lets its caller in with the writer between its transactions: a `Pause` is the caller's callback and how often it wants a turn (the daemon's is `PAUSE_EVERY`, 250 ms, in which it records the hook events waiting); a `Pacer` tells the write when a turn is due and gives it; `wait_for` waits for a message while offering turns |
| `create_private_folder`, `open_private_file` | A folder (with missing parents) or a file created or opened readable by its owner alone on Unix (0700, 0600), narrowing a wider mode found; the daemon's run files use them too |
| `Reader::open` | A read-only connection that can be open beside the writer |
| `Queries` | Reads shared by both: meta values, files, the path of every file (sorted by the bytes of the path, as Rust sorts strings), every file and every folder with its node, every symbol's and reference's name with its file's language (a symbol's kind and whether it is top-level, a reference's kind, line, definition and target), every symbol's node, the nodes edges of one type start from, how many files with a language and none of some flags there are and how many lines they hold, a node's search tokens, symbols, references, nodes, edges and row counts. For updates of a few files: the first symbols of one name in some languages, the references that go by a name, the files holding a reference of a name that goes through an import, the files holding ties the SCIP index gave into some files and every such tie, the references of one file, what each file passes on, a file's node by path, the file a node stands for, and whether any file is stored below a folder. For co-change: the miner's state, the files that count (neither generated nor vendored) by path with their nodes, and the commit counts and pairs of some files |
| `InterestRow`, `Queries::interest_of`, `Queries::file_interest_from` | A node's interest: the score it had at the task it was last touched at (`last_seq`), and the tier that score put it in. A score is kept as of a task, not as of now, so the decay each task brings is applied where it is read instead of to every row at every task. `interest_of` reads the nodes asked for; `file_interest_from` reads every file node whose stored score is at least a threshold, which is every file that could still stand above it, decay never raising a score. A node's interest goes with the node |
| `FileCommitsRow`, `PairRow`, `CountedFile`, `GitStateKey` | A file's commit counts, two files' co-change counts (the file with the smaller id first) with the newest commit they share, a file that counts with its node, and the miner's state: the commit mined last, its sequence number and when the last full mining was written. A file's counts go with the file |
| `SessionRecord`, `TaskRecord`, `EventRecord` and their rows | Agents' sessions, the tasks their prompts start and the events their hooks or the file watcher report: ids, kinds, paths, line ranges, times, sizes and exit codes, an event's other facts as typed JSON. `Queries` reads a session, a task, a session's latest open task, a task's or a session's events in order, and the events after a number |
| `FileIndex` and the other records | What the writer accepts and reports, a file's rows with what it passes on of its imports (`PassedOn`). A symbol keeps its id, its graph node and the node's edges across edits while its qualified name and kind stay the same. Row ids are never reused, so an id kept from an earlier read names its old row or nothing. A reference can only point at an existing symbol and an edge can only join existing nodes; an edge goes with either of its nodes |

Every graph node is written with its search row and removed with it: the identifier tokens of
its path for a file or folder, of its name and qualified name for a symbol, lower-case words
joined by spaces, in the full-text index with the node's id as its row id. A node with no word
(the root folder) has no row.

## Settings and files

Every connection uses write-ahead logging, `synchronous=NORMAL`, foreign keys, a 2-second busy
timeout, up to `store.mmap_bytes` of memory mapping (256 MiB by default) and in-memory temporary
tables. Each setting is read back after it is set.

A thread the writer starts with it copies the log into the database file every second, with a
passive checkpoint on a connection of its own, which waits for no reader and keeps the writer
from waiting for it. The writer's own commits checkpoint only once the log passes 16,384 pages
(64 MiB, `wal_autocheckpoint`): the thread has then left them about a second of writes to copy,
and a checkpoint that has copied everything lets the next transaction start the log afresh, which
a writer that commits without pause never leaves the thread time to allow. A checkpoint of the
thread that fails is tried again a second later and counted (`Writer::checkpoint_failures`);
nothing committed is lost. The thread stops before the writer's connection closes, and that last
close copies the rest and removes the log.

On Unix, the store's folders and files are readable by their owner only (modes 0700 and 0600),
including ones it finds with a wider mode. On Windows they inherit the access list of the folder
they are created in.

## Tests

```sh
cargo test -p cortex-store
```

Expected values come from the table definitions typed into the schema test and from records built
by hand. The no-code test writes the records of a small source file and then searches the database
file's bytes for every line of that source. The interest tests write scores by hand, read them back
for chosen nodes and for the files that could still be landmarks, replace one in place, and remove
a file to watch its interest go with it.
