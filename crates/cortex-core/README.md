# cortex-core

The types every other Cortex crate shares: repository paths, the repository id, content hashes,
the languages Cortex parses, the names the index may store, the kinds of record it keeps, scale
modes and settings.

## What it must never do

- Read or write files, open sockets, start processes or depend on a crate that does. Every other
  crate depends on this one, including the hook client, which must start fast.
- Accept a path that leaves the repository or a stored name that spans lines: these types are
  where those rules hold.

## Entry points

| Item | What it is |
| --- | --- |
| `RepoPath` | A path relative to the repository root with `/` separators; `from_native` converts a platform path inside the root, including Windows verbatim (`\\?\`) spellings |
| `RepoId` | The 16-character id of a repository on this machine, from the BLAKE3 hash of its canonical root |
| `ContentHash`, `ShortHash` | BLAKE3 hashes of 32 bytes (file contents) and 16 bytes (symbols, anchors) |
| `Language`, `Tier` | The 21 languages and formats with their stored names; Tier A gets symbols, Tier B keys or headings |
| `Identifier` | A stored name: one line, at most 256 bytes, no control characters |
| `SymbolKind`, `RefKind`, `Resolution`, `NodeKind`, `EdgeType`, `FileFlags` | The kinds of record the index keeps, with the names, numbers and bits stored for them; a `Resolution` also gives the confidence of a reference resolved that way |
| `identifier_tokens` | Splits a name or a path into the lower-case words a search matches: at anything but a letter or a digit, at a capital after a small letter or a digit, and at the last capital of a run when a small letter follows (`parseHTTPRequest` gives `parse`, `http`, `request`) |
| `ScaleMode` | `tiny`, `small`, `medium` or `huge`: how fully a repository of a given size is served |
| `Config`, `load_config` | The settings in effect: defaults, then `~/.cortex/config.toml`, then `.cortex/config.toml`; unknown keys warn, unusable values are errors naming the file and key. Keys: `mode`, `index.max_file_bytes`, `index.extra_ignores`, `index.parse_timeout_ms`, `daemon.idle_shutdown_minutes`, `store.mmap_bytes` |

## Tests

```sh
cargo test -p cortex-core
```

Expected values come from hand-worked examples and from the published BLAKE3 test vectors.
