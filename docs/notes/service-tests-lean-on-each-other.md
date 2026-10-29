---
title: Service tests lean on each other
type: note
summary: Read's and Write's tests use each other's code to set up and read back, and Check's tests use Write's test kit, so no service's tests stand alone; each crate should own its tests, so this is open.
status: evolving
---

# Service tests lean on each other

## Where it stands

Each crate's own tests should cover and catch its own code, as for a library published on crates.io. The three services, `sungma-check`, `sungma-read` and `sungma-write`, don't. Their tests take other services' code as dev-dependencies:

- **Read uses Write.** Read's HTTP tests (`crates/sungma-read/tests/common/mod.rs`) merge Write's router into Read's, so a test writes through Write and reads back through Read. Read's context tests (`crates/sungma-read/tests/read.rs`) write their rows with Write's `RowWriter` and `NamespaceWriteService`.
- **Write uses Read.** Write's HTTP tests (`crates/sungma-write/tests/common/mod.rs`) merge Read's router into Write's, to read back what a write did. `crates/sungma-write/tests/main.rs` reads through Read's `NamespaceReadService`.
- **Check uses Write.** Check's tests use Write's test kit, and Write's `testkit` feature depends on `sungma-check`.
- **All three use `sungma-node`**, which depends on Check and Write, for the names, the interner and the writer.

Cargo allows the cycles because they're dev-dependencies. The mutants still pass per crate. But a service's tests then depend on another service behaving, so a bug in Write can fail Read's tests, and when the services move to their own repos, their tests can't move with them.

## Options

1. **Set up through the table.** A service's tests write the rows they need straight to the table, in the shared layout, and read back what a write stored straight from the table. Each service tests itself against the layout alone. The cost: tests build rows by hand, so a layout helper belongs with the layout, not with Write.
2. **A test kit of fixtures.** One crate, depended on by every service's tests, that builds tables already holding facts, namespaces and names. It removes the cycles, but it's a crate that knows every service's rows.
3. **Round trips in their own place.** Tests that write through Write and read back through Read are integration tests of the system, not of either crate. They belong in a test crate of their own, or in a test against the running services.

Options 1 and 3 together keep each crate's tests its own and still test the round trip once.

## Open

- Which option, or which mix.
- Whether `sungma-node` stays once each service wires its own context.
