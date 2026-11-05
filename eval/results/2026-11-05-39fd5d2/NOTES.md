# Notes on this run

This run's copies were not fresh. The testbed folder they were copied from held a `.cortex/` folder
with an index an older build of Cortex had made of it, and the runner copied it along. Each copy's
daemon kept that index for every file whose contents had not changed, so the symbols and references
it retrieved from were partly those of the older indexer. The index held no sessions, tasks or work
orders, so nothing learned reached these runs from outside them.

The runner now leaves `.cortex/` behind when it copies a testbed, and refuses to reuse a run folder.
The run that measures the suite is the next one in `eval/results/`.
