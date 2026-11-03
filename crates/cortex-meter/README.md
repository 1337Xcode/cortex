# cortex-meter

What agents' work costs in tokens, read from the transcripts the agents write as they work.

## What it must never do

- Keep or log anything a transcript says beyond the counts, the model's name and the time: no
  prompt, no answer, no tool's input or output.
- Read a transcript twice over: a tail reads each whole line once, from where it stopped.
- Guess a count: a line missing one of its counts is passed over, not taken as zero.

## Entry points

| Item | What it is |
| --- | --- |
| `Tail`, `Turn`, `MAX_LINE_BYTES`, `MAX_RECENT_RESPONSES` | Reads a transcript from the byte offset where the last read stopped, the whole lines within a given number of bytes a read (a line longer than that read on to its end, as far as 64 MiB), and returns the responses those lines report, in order: a line cut short waits until it is whole. Each response is a `Turn` numbered by its order among the transcript's responses, from 1, with its time in UTC milliseconds, model, counts and how it was served. A response written on several lines, one per content block, counts once, and so does a side conversation's line that replays it under another request: a message id names one response. A later copy with more tokens comes again under the same number, its counts and service replacing the earlier ones. A response of the `<synthetic>` model counts nothing. The latest 64 responses are remembered for this. A file shorter than where the last read stopped was written again and is read from its start, numbered as before. A line longer than 64 MiB is passed over |
| `TranscriptFormat`, `format_of`, `Claude` | How an agent writes its transcript's lines, and the folders it keeps them in, one format per agent: Claude Code's for now, kept in `~/.claude/projects` and, when `CLAUDE_CONFIG_DIR` holds an absolute path, in its `projects` folder too. A Claude Code response is an `assistant` line whose message carries `usage`: the message id tells it apart, `timestamp` dates it, and the counts are the uncached input, the cache writes (the five-minute and one-hour writes added up when both are given, with the one-hour part kept apart; otherwise the flat count, its split unknown), the cache reads, the output and the reasoning within it (`output_tokens_details.thinking_tokens`). How it was served is read from `usage.service_tier`, `speed` and `inference_geo`, each only when it is text that is a name; a value of another type is unknown and leaves the line readable |
| `Response`, `Usage`, `Service`, `SYNTHETIC_MODEL` | One response as a line reports it, its counts, and how it was served: its service tier, speed and where it ran, each kept only when the line names it with a name |
| `utc_millis` | A transcript's time, `2026-11-02T23:04:17.241Z`, as UTC milliseconds; anything else is no time |
| `MeterError` | A transcript that could not be measured or read |

## Tests

```sh
cargo test -p cortex-meter
```

The tests read three real Claude Code transcripts under `tests/fixtures/claude/2.1.316/transcripts`,
reduced to the fields a usage reader looks at and scrubbed: every response's counts are written out
by hand from the fixture, and for one session their sums are checked against the running totals
Claude Code itself wrote in the same file. They read a transcript whole, cut short in the middle of
a line, in pieces cut anywhere (a property: the pieces give what the whole gives), written again
shorter, with a copy of a response raised and lowered, with a replayed and a synthetic response, with a line missing a count,
past the number of responses remembered, and with a line longer than a read.
