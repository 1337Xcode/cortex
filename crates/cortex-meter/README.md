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
| `Tail`, `Turn`, `MAX_READ_BYTES`, `MAX_RECENT_RESPONSES` | Reads a transcript from the byte offset where the last read stopped, at most a given number of bytes a read (64 MiB for the daemon), and returns the responses its new whole lines report, in order: a line cut short waits until it is whole. Each response is a `Turn` numbered by its order among the transcript's responses, from 1, with its time in UTC milliseconds, model and counts. A response written on several lines, one per content block, counts once, and so does a side conversation's line that replays it under another request: a message id names one response. A later copy with more tokens comes again under the same number, its counts replacing the earlier ones. A response of the `<synthetic>` model counts nothing. The latest 64 responses are remembered for this. A file shorter than where the last read stopped was written again and is read from its start, numbered as before. A read of the whole allowance with no line ending passes that line over |
| `TranscriptFormat`, `format_of`, `Claude` | How an agent writes its transcript's lines, one format per agent: Claude Code's for now. A Claude Code response is an `assistant` line whose message carries `usage`: the message id tells it apart, `timestamp` dates it, and the counts are the uncached input, the cache writes (the five-minute and one-hour writes added up when given, with the one-hour part kept apart), the cache reads, the output and the reasoning within it (`output_tokens_details.thinking_tokens`) |
| `Response`, `Usage`, `SYNTHETIC_MODEL` | One response as a line reports it, and its counts |
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
