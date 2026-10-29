![Sungma](https://raw.githubusercontent.com/marklauter/Sungma/main/assets/logo/sungma-sml.png "Sungma")

# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a Zanzibar-style relationship-based access control (ReBAC) service. It answers one question: is subject S in the subjectset `namespace:resource#relation` at revision R? It answers with the facts that prove a yes, and can replay any recorded decision at its revision.

## Why revisions

Most authorization code checks a permission in one query and acts in another. That gap is a time-of-check to time-of-use (TOCTOU) bug: between the two, a grant can be revoked, a replica can lag or a cache can go stale. The action then goes ahead on a decision nobody can reconstruct.

Sungma answers every check at a revision, and the answer names it:

- A write returns the revision it was stamped with. A client that stores content alongside that revision, and passes it on later checks, is never judged against a snapshot older than the content. That is Zanzibar's answer to the new enemy problem, where a user removed from a folder sees a document added after the removal.
- A decision records its revision and the facts that proved it, so replay shows what was true when it was made, not what is true now.

A revision doesn't close the gap between a check and the action that follows it. It names the snapshot the action relied on, so a stale decision can be detected rather than silently trusted. Writes that overlap can still race; [the clock spec](docs/specs/clock.md#overlapping-writes) says how.

## Status

A proof of concept. The evaluator, decisions with grounds and replay, namespace documents, versioned namespaces, the revision clock and a DynamoDB table work. The HTTP API checks access and reads and writes facts and namespaces. [docs/roadmap.md](docs/roadmap.md) lists what comes next.

## Crates

Each context has its own readers, writers and shapes, over one key-value table and a small shared kernel.

- `sungma`: the shared kernel. Names, ids and revisions, the fact graph, namespaces and rewrites, and namespace versions with their epochs.
- `sungma-table`: the key-value table, which knows nothing of what its items mean: a get by key, a query of a partition, a unit of work that commits whole or not at all, and sequences. Holds the memory table, for tests, and the tests every table passes.
- `sungma-dynamodb`: the table on DynamoDB.
- `sungma-schema`: where facts, namespace versions and the head live on the table, and what each row holds.
- `sungma-names`: the names context. Names and their interned ids, the interner, and resolving a caller's names.
- `sungma-write`: the write context. Fact and namespace writes, judged by the epoch rules and committed as one unit of work.
- `sungma-check`: the Check context. Resolves a request's names, decides it at a revision and appends an audit record; closure evaluation and replay.
- `sungma-read`: the API's reads of facts and namespace documents at a snapshot.
- `sungma-clock`: the revision clock, with NTP, NTS and platform clocks.
- `sungma-lang`: namespace and fact documents: parsing and printing.
- `sungma-node`: one node's contexts wired over one table.
- `sungma-http`: what the HTTP services share. `sungma-check`, `sungma-read` and `sungma-write` each serve their context over HTTP, on axum, with the `server` feature.

## Docs

- [Architecture](docs/architecture.md): Sungma, its neighbors and its services.
- [Sample](docs/sample.md): files, folders and groups, with a trace of one check.
- [Glossary](docs/glossary): the domain terms.
- [Specs](docs/specs): namespace documents, fact documents, namespace writes, the revision clock, one timeline and the table.
- [Notes](docs/notes): open work and the current thinking on it.
- [C4 diagrams](docs/c4): the services and their containers, in Excalidraw.

## Development

`cargo test --workspace` runs the tests. The `sungma-dynamodb` tests need DynamoDB Local, which `scripts/dynamodb.sh` starts. Each crate's own tests cover its code. Line coverage stays above 95%, measured with `cargo llvm-cov`.

`rust-toolchain.toml` installs the stable toolchain with clippy, rustfmt and rust-analyzer. Enable the hooks once per clone. Before each commit they check formatting, lints, docs and tests; before each push, coverage and the dependency audit, as CI does. The push checks need `cargo-llvm-cov` and `cargo-audit`. Mutation testing of a branch's changes runs in CI only.

```sh
git config core.hooksPath .githooks
```

`fuzz/` holds `cargo-fuzz` targets for the namespace and fact document parsers, which CI runs for a minute each, and for the NTP and NTS parsers, which it runs for 30 seconds each. Locally they need the nightly toolchain:

```sh
cargo +nightly fuzz run document
cargo +nightly fuzz run fact
```

### Run the check API

```sh
scripts/dynamodb.sh
export SUNGMA_DYNAMODB_URL=http://localhost:8000
F=crates/sungma/tests/fixtures
# The Write service creates the table and seeds it.
cargo run -p sungma-write --features server -- $F/docs.facts.json $F/namespaces/*.json &
# Once it listens:
cargo run -p sungma-check --features server &

curl -d '{"set": "file:design.md#viewer", "identity": "alice"}'   -H 'content-type: application/json' localhost:8080/check
# {"request_id":"sungma-1","verdict":"allowed","zookie":{"revision":16}}
```

The subject is an `identity` or a `subjectset`; an optional
`"zookie": {"revision": n}` asks for a revision at least that fresh.
`SUNGMA_ADDR` sets the listen address.
