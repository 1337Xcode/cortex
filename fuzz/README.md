# cortex-fuzz

Fuzz targets for the two parsers that read input Cortex does not control: what an agent writes
to the hook's stdin, and the frames that cross the daemon's socket. Each target must turn any
bytes into a result or an error, never a panic, a hang, or an allocation past a cap.

| Target | What it feeds |
| --- | --- |
| `hook_stdin` | The bytes to the Claude Code adapter's `normalize`, as each event Cortex registers, with a working folder that is always in a repository and again in a linked working tree, so no path in the input reaches the file system |
| `frame_decode` | The bytes to `read_frame` as a stream of requests and of answers, and to `decode` as one frame's body |
| `extract_source` | Arbitrary bytes read as source, the first byte choosing which of the 21 languages, through the extractor that every file an agent touches goes through |

The crate is a workspace of its own, built on nightly by
[cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html), on Linux or macOS. From this
folder, with the recorded hook payloads as the starting corpus of the first target:

```bash
cargo +nightly fuzz run hook_stdin corpus/hook_stdin ../tests/fixtures/claude/2.1.312 -- -max_total_time=600
```

```bash
cargo +nightly fuzz run frame_decode corpus/frame_decode -- -max_total_time=600
```

```bash
cargo +nightly fuzz run extract_source corpus/extract_source -- -max_total_time=600 -timeout=25
```

The source extractor is slower than the other two by a wide margin, since every input is parsed by
a real grammar, so give it the time its runs need rather than expecting their counts. Its corpus
starts best from real files: write each as one byte naming the language (the index into the
language list, modulo its length) followed by the file.

libFuzzer adds the inputs it finds to the first folder named; `corpus/`, `artifacts/` and `target/`
are left out of git. An input that crashes a target is saved in `artifacts/<target>/`, and
`cargo +nightly fuzz run <target> <file>` runs it again.
