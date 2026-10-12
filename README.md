![Sungma](https://raw.githubusercontent.com/marklauter/Sungma/main/assets/logo/sungma-sml.png "Sungma")

# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a Zanzibar-style relationship-based access control (ReBAC) service. It answers one question: is subject S in the subjectset `namespace:resource#relation` at revision R? It answers with the facts that prove a yes, and can replay any recorded decision at its revision.

## Status

A proof of concept. The evaluator, decisions with grounds and replay, namespace documents and the revision clock work; storage is in memory only, and there is one HTTP route, `POST /check`. [docs/roadmap.md](docs/roadmap.md) lists what comes next.

## Crates

- `sungma`: the kernel. Names, ids, the fact graph, namespaces and rewrites, the read ports, closure evaluation, decisions and replay.
- `sungma-check`: the Check service. Resolves a request's names, decides it and appends an audit record.
- `sungma-write`: the write ports and id minting, and the write pipeline to come.
- `sungma-memory`: in-memory adapters for the storage ports.
- `sungma-clock`: the revision clock, with NTP, NTS and platform clocks.
- `sungma-lang`: namespace and fact documents: parsing, printing and loading.
- `sungma-api`: an axum shell with `POST /check`.

## Docs

- [Architecture](docs/architecture.md): how the code fits together, with a worked sample.
- [Glossary](docs/glossary): the domain terms.
- [Specs](docs/specs): namespace documents, fact documents, the revision clock and one timeline.
- [Notes](docs/notes): open work and the current thinking on it.
- [C4 diagrams](docs/c4): the services and their containers, in Excalidraw.

## Development

`cargo test --workspace` runs the tests. Line coverage stays above 95%, measured with `cargo llvm-cov`.

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
F=crates/sungma/tests/fixtures
cargo run -p sungma-api -- $F/docs.facts.json $F/namespaces/*.json

curl -d '{"set": "file:design.md#viewer", "identity": "alice"}'   -H 'content-type: application/json' localhost:8080/check
# {"request_id":"sungma-1","verdict":"allowed","zookie":{"revision":16}}
```

The subject is an `identity` or a `subjectset`; an optional
`"zookie": {"revision": n}` asks for a revision at least that fresh.
`SUNGMA_ADDR` sets the listen address.
