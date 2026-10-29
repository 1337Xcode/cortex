# cortex-core

The types every other Cortex crate shares: repository paths, the repository id, content hashes,
the languages Cortex parses, the names the index may store, scale modes and settings.

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
| `ScaleMode` | `tiny`, `small`, `medium` or `huge`: how fully a repository of a given size is served |
| `Config`, `load_config` | The settings in effect: defaults, then `~/.cortex/config.toml`, then `.cortex/config.toml`; unknown keys warn, unusable values are errors naming the file and key |

## Tests

```sh
cargo test -p cortex-core
```

Expected values come from hand-worked examples and from the published BLAKE3 test vectors.
