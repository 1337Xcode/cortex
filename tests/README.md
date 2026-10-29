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

`fixture_integrity` checks that every fixture parses and names the event its file name says; that
every event and tool the adapters handle has a fixture for each captured agent version, holding the
fields its name promises (a tool, a source, a test run that passed or failed, a read with a line
range); that every version folder is listed; that the fixture folder holds only payloads and
READMEs; and that nothing in it carries a real path, id, credential, opaque hash, user or machine
name, email address or CRLF line ending, in any key or value. The required fixtures and their
fields live in `src/required.rs`, the scrub rules in `src/scrub.rs` and the home-folder rules in
`src/homes.rs`, each with its own tests. Adding an event or a version means adding its files and
one entry in the `REQUIRED` table.
