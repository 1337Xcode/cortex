# cortex-core

The types every other Cortex crate shares: repository paths, the repository id, content hashes,
the languages Cortex parses, the names the index may store, the kinds of record it keeps, scale
modes and settings.

## What it must never do

- Open sockets, start processes, read a repository's files, or depend on a crate that does. Every
  other crate depends on this one, including the hook client, which must start fast. Its calls to
  the file system resolve a folder to its canonical form (`canonical_root`), so that every crate
  names a repository the same way, and create the user's private folders and files and rotate the
  logs in them, so that the hook and the daemon do both the same way.
- Accept a path that leaves the repository or a stored name that spans lines: these types are
  where those rules hold.

## Entry points

| Item | What it is |
| --- | --- |
| `RepoPath` | A path relative to the repository root with `/` separators; `from_native` converts a platform path inside the root, including Windows verbatim (`\\?\`) spellings |
| `canonical_root` | A folder in its canonical form: absolute, links and `..` resolved, names spelled as stored, no Windows verbatim prefix; every spelling of one folder gives one result |
| `RepoId` | The 16-character id of a repository on this machine, from the BLAKE3 hash of its canonical root |
| `ContentHash`, `ShortHash` | BLAKE3 hashes of 32 bytes (file contents) and 16 bytes (symbols, anchors) |
| `Language`, `Tier` | The 21 languages and formats with their stored names; Tier A gets symbols, Tier B keys or headings |
| `Identifier` | A stored name: one line, at most 256 bytes, no control characters |
| `SymbolKind`, `RefKind`, `Resolution`, `NodeKind`, `EdgeType`, `FileFlags` | The kinds of record the index keeps, with the names, numbers and bits stored for them; a `Resolution` also gives the confidence of a reference resolved that way |
| `identifier_tokens` | Splits a name or a path into the lower-case words a search matches: at anything but a letter or a digit, at a capital after a small letter or a digit, and at the last capital of a run when a small letter follows (`parseHTTPRequest` gives `parse`, `http`, `request`) |
| `ScaleMode` | `tiny`, `small`, `medium` or `huge`: how fully a repository of a given size is served |
| `Event` | One hook call, normalized: the agent, its session, the point in its loop (`HookKind`), the tool (`ToolKind`), what the tool aimed at (`ToolTarget`: a repository path, checked `LineRange`s, a pattern, a `ShellCommand` cut to 1,000 characters) and how it ended (`ToolResult`: exit code, success, interruption, output size), never file contents or tool output; serialized with names for every kind and checked when read |
| `SessionStatus`, `TaskOutcome`, `EventKind`, `EventSource`, `EventMeta`, `PromptExcerpt` | What the store keeps of agents' activity: a session's status, a task's outcome, an event row's kind and source, the facts an event keeps besides its path and lines (tool, exit code, success, interruption, output size, why a session started or ended, what started a compaction, a subagent's type), and the first 200 characters of a prompt kept only on request |
| `Ulid` | A sortable unique id: UTC milliseconds, then 80 bits from the BLAKE3 hash of a seed the caller gives; written as 26 characters, and only that spelling is read back |
| `unix_millis` | A clock time as UTC milliseconds since 1970, the unit the store and the protocol use; a clock set before 1970 reads as 0 |
| `rotate_log` | Makes room for a new log: each older file moves one place (`.1` to `.2`, the oldest kept replaced) and the log moves to `.1` |
| `create_private_folder`, `open_private_file`, `append_private_file` | Folders and files only the user can read: mode 0700 and 0600 on Unix, whatever the umask, a wider mode narrowed; on Windows the access list they inherit |
| `one_line` | Text for a log or a report a person reads: its first characters up to a limit, each control character and each character that reorders or hides text (bidirectional controls, zero-width characters, the byte order mark) written as a space, so that no text that reached it can split the line, steer a terminal or read differently from what it holds |
| `ipc` | The messages the hook, the CLI and a daemon exchange (`Request`, `Response`, the `Hello` handshake), framed as a 4-byte little-endian length and at most 1 MiB of JSON (`encode`, `decode`, `read_frame`, `write_frame`); `Protocol` (majors must match), `CortexVersion` (ordered as semantic versioning orders versions) and `is_older_than_this_build` (false for a version that does not read, so no build replaces what it cannot rank). The envelope, `Hello`, `Mismatch` and `Shutdown` never change shape |
| `UserFolder`, `RunFiles`, `Endpoint` | `~/.cortex` and what it holds: `config.toml`, `logs/`, and per repository id the daemon's socket (`run/<id>.sock`), named pipe (`cortex-<id>`), lock and process-id files; the endpoint a daemon listens on here, its named pipe on Windows and its socket elsewhere |
| `Config`, `load_config` | The settings in effect: defaults, then `~/.cortex/config.toml`, then `.cortex/config.toml`; unknown keys warn, unusable values are errors naming the file and key. Keys: `mode`, `index.max_file_bytes`, `index.extra_ignores`, `index.parse_timeout_ms`, `index.crawl_files_per_second`, `daemon.idle_shutdown_minutes`, `store.mmap_bytes`, `privacy.store_prompt_excerpt` |

## Tests

```sh
cargo test -p cortex-core
```

Expected values come from hand-worked examples and from the published BLAKE3 test vectors.
