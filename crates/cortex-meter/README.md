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
| `Tail`, `Turn`, `MAX_LINE_BYTES`, `MAX_RECENT_RESPONSES` | Reads a transcript from the byte offset where the last read stopped, the whole lines within a given number of bytes a read (a line longer than that read on to its end, as far as 64 MiB), and returns the responses those lines report, in order: a line cut short waits until it is whole. Each response is a `Turn` numbered by its order among the transcript's responses, from 1, with its time in UTC milliseconds, model, counts, how it was served and a hash of its message id, which a copy of it in another transcript keeps. A response written on several lines, one per content block, counts once, and so does a side conversation's line that replays it under another request: a message id names one response. A later copy with more tokens comes again under the same number, its counts and service replacing the earlier ones. A response of the `<synthetic>` model counts nothing. The latest 64 responses are remembered for this. A file shorter than where the last read stopped was written again and is read from its start, numbered as before. A line longer than 64 MiB is passed over |
| `TranscriptFormat`, `format_of`, `Claude` | How an agent writes its transcript's lines, and the folders it keeps them in, one format per agent: Claude Code's for now, kept in `~/.claude/projects` and, when `CLAUDE_CONFIG_DIR` holds an absolute path, in its `projects` folder too. A Claude Code response is an `assistant` line whose message carries `usage`: the message id tells it apart, `timestamp` dates it, and the counts are the uncached input, the cache writes (the five-minute and one-hour writes added up when both are given, with the one-hour part kept apart; otherwise the flat count, its split unknown), the cache reads, the output and the reasoning within it (`output_tokens_details.thinking_tokens`). How it was served is read from `usage.service_tier`, `speed` and `inference_geo`, each only when it is text that is a name; a value of another type is unknown and leaves the line readable |
| `PriceTable`, `ModelPrices`, `Rates`, `Billed`, `Cost`, `Unpriced`, `SHIPPED_PRICES`, `load_or_write`, `PricingError` | Estimates what a response cost from a table of prices in US dollars per million tokens: the table Cortex ships (`pricing.toml`, copied from Anthropic's pricing page with the date it was read), written to the user's file the first time one is needed (whole, private to the user, and linked into place so that it never replaces a file already there and no reader sees it half written) and read from there after, the user's prices winning. A model's standard prices, its fast mode prices where it has them, a multiplier for US-only inference where it offers it, and prices for long prompts where it charges them apart; a dated model id takes its undated prices. Costs are whole picodollars, so they add up exactly. A response is priced only when the table knows everything its price depends on (its model, the standard tier, its speed, where it ran, and how its cache writes split); otherwise it has no estimate and says why. Tokens only: per-use charges for server tools are not counted |
| `Response`, `Usage`, `Service`, `SYNTHETIC_MODEL` | One response as a line reports it, its counts, and how it was served: its service tier, speed and where it ran, each kept only when the line names it with a name |
| `Activity`, `receipt`, `SessionReceipt`, `TaskReceipt`, `Tally`, `Estimate` | Adds up what one session's tasks took. A task runs from its prompt to its stop, or to the next task's start when it never stopped; each response, the session's and its subagents', belongs to the task whose span holds its time, and the rest are counted apart. Per task: turns, tool calls, every input token (read fresh, written to the cache and read from it), the uncached part, cache reads and writes, output, and the cost summed over the responses the price table can price, the others counted by the reason they have none; the session's tool calls, outside its tasks too; and whether its agent's transcripts are read at all (`is_metered`), since a session whose agent has no reader has no usage that says nothing. What Cortex added is counted from its injections that reached the agent (`InjectionRecord::was_delivered`): each task's work orders, the session's primers, and the two together; one the gate kept back, held out or not written in time adds nothing |
| `Precision` | Of the work orders that reached the agent and whose tasks ended, how many were labelled and how many of those were used; its share, none before any was labelled |
| `weighed`, `compare`, `Weighed`, `Comparison`, `Compared`, `BOOTSTRAP_RESAMPLES` | The holdout comparison. `weighed` takes the tasks of session receipts that ended with an outcome and whose arm was decided, by arm, each with whether it succeeded and, when its agent's transcripts are read, its cumulative input, uncached input, turns and tool calls. `compare` sets the treated arm against the held-out one once each holds 30 such tasks: the success rates, and per successful task the medians of the four, each with the difference (treated minus held out) and its 95% percentile bootstrap interval, each arm resampled 2,000 times from a generator seeded by a hash of the values, so the same tasks always give the same interval; no medians when an arm has no successful task whose usage was read |
| `Arms`, `MIN_TASKS_PER_ARM` | How many of a repository's tasks were held out from Cortex and how many were not, and whether each arm has the 30 a comparison needs |
| `utc_millis`, `utc_text` | A transcript's time, `2026-11-02T23:04:17.241Z`, as UTC milliseconds, and back; anything else is no time |
| `MeterError` | A transcript that could not be measured or read |

## Tests

```sh
cargo test -p cortex-meter
```

The tests read three real Claude Code transcripts under `tests/fixtures/claude/2.1.316/transcripts`,
reduced to the fields a usage reader looks at and scrubbed: every response's counts are written out
by hand from the fixture, and their sums are checked against the running totals Claude Code
itself wrote: equal for a session without `/compact`, and for one with it short by exactly the
call that summarized the session, which no transcript line records and so is not counted. They read a transcript whole, cut short in the middle of
a line, in pieces cut anywhere (a property: the pieces give what the whole gives), written again
shorter, with a copy of a response raised and lowered, with a replayed and a synthetic response, with a line missing a count,
past the number of responses remembered, and with a line longer than a read. The pricing tests
check every model of the shipped table against its row copied by hand from the pricing page, work
costs out by hand (the cache's two lifetimes, fast mode in the United States, a long prompt either
side of its threshold, reached by input alone and by cache reads, a dated id), give every reason a
response has no estimate, refuse files that are not price tables (unknown keys, prices and
multipliers that are no price or too small to keep, dates that are no day, text that is not
UTF-8, a file too large), and write the shipped table once, privately and leaving nothing else
behind, then read the user's prices as they are. The receipts tests add up a session by hand:
responses at a task's start and end, between tasks, before any, in a task left open and from a
subagent, each unpriced reason, no price table, tasks listed out of order, tool calls in and
outside tasks, and each task's arm once decided.
