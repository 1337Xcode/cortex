# cortex-retrieve

What Cortex hands an agent before it starts searching. So far it holds what a prompt points at and
what its words find, what the session has been doing, the graph that activation spreads over, the
spreading itself, the gate's rule, and the reading of a file that lends its lines only when it is
the file the index read.

## What it must never do

- Inject anything it is not sure of: its gate stays silent unless independent signals agree, and
  silence is what a missing signal gives, never a guessed span.
- Show a line number or a line of code it has not just checked against the file on disk.
- Let the order rows came in, a hash map's order or a clock reach what it renders.
- Write the store: it reads, and the daemon's single writer records what it rendered.

## Entry points

| Item | What it is |
| --- | --- |
| `PromptTerms`, `Mention` | What a prompt points at: the words a search looks up (its identifier tokens, split as the index splits names, less the words that name no place, digits alone and single letters), and the mentions that may name a file or a symbol exactly: a path with the line a stack frame or traceback gives, a name written as code (an underscore, a capital inside, digits among letters, parts joined by `::`, backticks), or both for a word like `service.ts` or `Config.load` |
| `Graph`, `NodeInput`, `LinkInput`, `Weighing`, `Standing`, `GraphError` | The graph of the store's nodes and links, as compressed sparse rows per kind of link, nodes numbered in id order. A link weighs its kind's weight from `retrieval.type_weights` times its strength in 0 to 1: a reference's confidence, a co-change confidence, a co-access link's weight faded by `learning.link_decay` to the latest task and taken over the most a link can weigh, a folder's hold on its files. Co-change links run the way history shows them; every other link both ways. A file is joined to each of its symbols both ways at the containment weight. Links of one kind between the same nodes add up; a link to itself, to a node not given, or of no weight is left out; a node given twice keeps the first of its kinds and files in their order, and a symbol whose file is no file node has none. A graph past 32-bit node numbers or link counts is refused (`GraphError`). `out_weight` is a node's links' sum over every kind, by which spreading divides them. Each node carries its standing: its interest brought forward by `learning.interest_decay` to the latest task, the tier that puts it in (a landmark as the last task to choose them left it), and whether a work order may point at it (not a generated or vendored file) |
| `lexical`, `LexicalMatch`, `FileMatch`, `MAX_HITS` | The prompt's words looked up by BM25: the 50 best rows, each a seed weighed by its score over the best's; a word more than one row in twenty holds (and more than twenty rows) is not looked up but still counts. A path or a name the prompt writes exactly, found in the repository, is a seed of weight 1: the file (by its path, the one path ending in it, or the one path an absolute path ends in), the symbol at its line, the symbols of a name five or fewer share. Each file's match is the share of the prompt's words it holds, weighed by inverse document frequency, over at least twice the weight of a word one row in a hundred holds; a named file matches fully; the top lexical score is the best file's |
| `session`, `SessionMatch`, `ReadSpan`, `EDITED_WEIGHT`, `READ_WEIGHT`, `SEEDED_READS` | What the session has been doing: the code its tasks edited, its subagents' edits included, seeded at 1; the code its latest 10 reads touched, its own, at 0.4; a node edited and read carries both. A stretch of lines counts for the symbols it touches, or its file when it touches none or names no lines. The files it edited, and every span it read since its context was last compacted, with the short hash of what the index held for the file then. A read or edit that failed counts for nothing |
| `read_graph`, `GraphSettings`, `RetrieveError` | The graph of everything the store holds, weighed by `retrieval.type_weights`, co-access links faded by `learning.link_decay` to the store's latest task, each node's standing as of that task |
| `spread`, `Activation`, `SPREAD_THRESHOLD`, `L1_TOLERANCE`, `MAX_PUSHES` | Spreading activation from seeds: an approximate personalized PageRank by local push, with the restart probability given (`retrieval.teleport`). A node is pushed while it has the threshold left to spread (first 1e-5), in the order nodes came to have it; when none has but more than 1e-3 is left in all, the threshold is lowered tenfold and pushing goes on, so a spreading that ends on its own is within 1e-3 (L1) of exact PageRank. It stops cut short after 50,000 pushes or at a deadline. A node never keeps more than its exact value, and what is left to spread bounds the shortfall |
| `decide`, `Features`, `Verdict`, `Rule`, `Shut`, `AGREEING_KINDS`, `LEXICAL_BAR` | The gate. The cold-start rule lets a work order through when two independent kinds of evidence support one top file and the prompt's best-matching file scores at least 0.5 (plus the self-check's raise), or when the session is plainly continuing its own work; otherwise it is silent and says why |
| `CheckedFile`, `Unchecked` | A repository file read whole, never through a link at its own path and never past a size limit, only when its BLAKE3 hash is the one the index stored, so a link among the folders above it cannot lend other bytes either; its lines numbered from 1 as the index counts them (`cortex_index::count_lines`) |

## Tests

```sh
cargo test -p cortex-retrieve
```

Weights are worked out by hand in each test's comment; a property checks that the same rows in any
order give the same graph. Spreading is checked against exact personalized PageRank, computed by
power iteration over a petgraph copy of each random graph (`tests/push.rs`); the gate against a
table of cases; reading against files changed, too large or not files at all.
