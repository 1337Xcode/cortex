# cortex-tests

Cross-crate tests and the fixtures they share.

## Fixtures

`fixtures/<agent>/<version>/` holds real hook payloads captured from each agent version, one file
per event (and per tool for tool events), scrubbed of paths and ids. Each folder's README names the
version, the capture date and how the payloads were captured and scrubbed.

## What it must never do

- Hold a fixture with a real path, user name, session id or any content not produced by the
  synthetic capture repository.
- Change a fixture to make a test pass. A new agent version gets a new folder.

## Tests

```sh
cargo test -p cortex-tests
```

`fixture_integrity` checks that every fixture parses, names the event its file name says, is
scrubbed, uses LF line endings, and that every event the adapters handle has a fixture for each
captured agent version. Adding an event or a version means adding its files and one entry in the
`REQUIRED` table.
