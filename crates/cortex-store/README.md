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
| `Writer::transaction`, `WriteTx` | One transaction: `replace_file`, `touch_file` (a file read again and found as stored: its new time and when it was read), `remove_file`, `set_resolutions`, `replace_edges`, `folder_node` (a folder's node, added on first use and kept until removed), `remove_folder_node`, `set_scale_mode`, for co-change `clear_cochange`, `add_cochange` (counts summed with the stored ones, a pair's newest shared commit kept), `set_git_state`, `remove_git_state` and `remove_edges_of_type`, for what is learned `replace_archetypes`, `replace_episode`, `replace_interest` and `set_edges` (one pair at a time, where `replace_edges` would drop a node's other edges of the type), for the tokens agents spent `replace_usage` (a row per response, replacing what its session and turn had) and `forget_usage_after` (a session's rows numbered past a turn, once its transcript was read again to its end), and for agents' activity `record_session` (added, or updated with the facts given while keeping its first start time, and live again), `end_session`, `start_task` (numbered one past every task in the repository), `end_task` (once), `set_task_outcome` (what became of a task that has ended, with the words that say why, replaced by a later judgement) and `append_event`, then `commit`. A write that fails is undone on its own, and the transaction keeps the writes before it. If SQLite rolls the whole transaction back after an error such as a full disk, every later write and the commit are refused |
| `Pause`, `Pacer`, `PAUSE_EVERY`, `wait_for` | How a long write (an index, a mining) lets its caller in with the writer between its transactions: a `Pause` is the caller's callback and how often it wants a turn (the daemon's is `PAUSE_EVERY`, 250 ms, in which it records the hook events waiting); a `Pacer` tells the write when a turn is due and gives it; `wait_for` waits for a message while offering turns |
| `create_private_folder`, `open_private_file` | A folder (with missing parents) or a file created or opened readable by its owner alone on Unix (0700, 0600), narrowing a wider mode found; the daemon's run files use them too |
| `Reader::open` | A read-only connection that can be open beside the writer |
| `Queries` | Reads shared by both: meta values, files, the path of every file (sorted by the bytes of the path, as Rust sorts strings), every file and every folder with its node, every symbol's and reference's name with its file's language (a symbol's kind and whether it is top-level, a reference's kind, line, definition and target), every symbol's node, the nodes edges of one type start from, how many files with a language and none of some flags there are and how many lines they hold, a node's search tokens (and many nodes' at once), symbols, references, nodes, edges and row counts. For updates of a few files: the first symbols of one name in some languages, the references that go by a name, the files holding a reference of a name that goes through an import, the files holding ties the SCIP index gave into some files and every such tie, the references of one file, what each file passes on, a file's node by path, the file a node stands for, and whether any file is stored below a folder. For co-change: the miner's state, the files that count (neither generated nor vendored) by path with their nodes, and the commit counts and pairs of some files |
| `ArchetypeRecord`, `Queries::archetypes`, `Queries::episodes` | Clusters of tasks that changed much the same files: the words their files and symbols are found by, which the words of a prompt are matched against, the files themselves, how many tasks the cluster covers and when it was last written. The clustering reads the newest records, oldest first, so the oldest task of a cluster is the one the others are compared against, and writes the clusters whole, replacing those stored before; whoever writes them keeps their lists bounded |
| `EpisodeRecord`, `Queries::episode_of_task`, `MAX_EPISODE_BYTES` | One task's record: the files it read before it changed anything with how often, the files and symbols it changed with how many lines and whether a declaration's own line was among them, the checks it ran with their runners and exit codes, what went wrong on the way (a runner whose check failed and later passed, the files it put back itself, the stretches it read over and over), and what became of it. Each part is JSON in a column of its own; the four together are refused past 2 KiB, so one task cannot fill the database. A record holds node ids, counts, line numbers, exit codes and runners' names, never a command, a path or code |
| `Queries::term_rows`, `Queries::search`, `Queries::files_named`, `Queries::symbol_nodes_named` | Lookups by name: how many search rows hold a word (from the index's vocabulary, `search_terms`, migration 0009); the rows that best match any of some words by BM25, best first and then by node, each word passed to the full-text index as a quoted phrase so no word changes what is asked; the files whose path is a name or ends in `/` and it, `LIKE`'s wildcards escaped; the nodes of the symbols of a name |
| `Queries::each_graph_node`, `Queries::each_edge`, `Queries::latest_task_seq`, `GraphNodeRow` | The whole graph, read row by row into what the caller builds from it so it is never held twice: every node with its kind, for a symbol its file's node, its interest row when it has one and for a file its flags; every edge in the order of its key; and the number of the latest task, which faded weights are brought forward to. A read stops at the caller's first error |
| `Queries::edges_among` | The edges of one type running from one of the nodes asked about to another of them, by source and then target: the links a task's work changes, and the structural edges that say which of the files it read an edited file names. An undirected edge is stored once, from the smaller of its two nodes, so a pair shows up once however the two are given |
| `UsageRow`, `Queries::usage_of_session` | The tokens one response of a session took: its place among its transcript's responses (the turn), its time, its model when it has a name for one, the uncached input, cache writes, cache reads, output and, when the transcript reports them, the cache writes made for an hour, the reasoning within the output, how the response was served (its service tier, speed and where it ran, as short names), and which response it is, as a 16-byte hash of the provider's id for the message. A count past what SQLite stores is refused, and a row for a session the store does not hold. `usage_of_session` reads a session's rows by turn, without a response a session that started earlier reports too (of two that started together, the one with the smaller id keeps it): a forked session's transcript copies its origin's responses with their ids, and they count once, for the origin |
| `InterestRow`, `Queries::interest_of`, `Queries::file_interest_from` | A node's interest: the score it had at the task it was last touched at (`last_seq`), and the tier that score put it in. A score is kept as of a task, not as of now, so the decay each task brings is applied where it is read instead of to every row at every task. `interest_of` reads the nodes asked for; `file_interest_from` reads every file node whose stored score is at least a threshold, which is every file that could still stand above it, decay never raising a score. A node's interest goes with the node |
| `FileCommitsRow`, `PairRow`, `CountedFile`, `GitStateKey` | A file's commit counts, two files' co-change counts (the file with the smaller id first) with the newest commit they share, a file that counts with its node, and the miner's state: the commit mined last, its sequence number and when the last full mining was written. A file's counts go with the file |
| `SessionRecord`, `TaskRecord`, `EventRecord` and their rows | Agents' sessions, the tasks their prompts start and the events their hooks or the file watcher report: ids, kinds, paths, line ranges, times, sizes and exit codes, an event's other facts as typed JSON. `Queries` reads a session, a task, a session's latest open task, a task's or a session's events in order, the events after a number, and newest first a session's latest edits (those of its tasks, so its subagents' too) and its own latest reads and compactions; a session's tasks are indexed by session, and edit rows by task apart from the other events (migration 0010). A task's row carries its outcome and the words that say why once it is judged |
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
a file to watch its interest go with it. The edge tests set one pair at a time to watch the node's other edges stay, read the edges with both ends among chosen nodes, and remove a file to watch its links go. The episode tests write a record of one task, read it back, replace it, and count by hand the files read that fit under the bound and the number that is refused. The cluster tests write clusters whole, read them back in order, replace them together, and read the newest records the clustering works from, oldest first. The search test reads many nodes' search rows at once, by node, with a node that has none left out.
