# Architecture

Sungma is a Zanzibar-style authorization service. It answers one question: is subject S in the subjectset `namespace:resource#relation` at revision R? This page is the high-level view, the first two levels of C4. The code, the [specs](specs) and the [README](../README.md)'s crate list hold the detail.

## Context

- **Client applications** ask Sungma who may do what, and tell it who may: they check access and write facts and namespaces.
- **NTS time servers** are public time servers, authenticated with NTS. They give the revision clock the bounds every revision is stamped within.
- **A DynamoDB table** holds facts, namespace versions and names. The adapter is built against DynamoDB Local.

## Containers

Sungma is a set of Rust services over one table. [c4-containers.excalidraw](c4/c4-containers.excalidraw) draws them.

- **Check**: is this subject in this subjectset at a revision? It answers with the facts that prove a yes, and records the decision.
- **Read**: facts by key, resource or subjectset, at one snapshot.
- **Write**: atomic writes of facts and namespaces, each stamped with one revision.
- **Expand**: the subject tree of a subjectset at a revision.
- **Watch**: events from the changelog, with heartbeat zookies.
- **Audit**: listens for Watch events; the audit log, replay and a UI.
- **Table**: facts, namespace versions and names, by key.

Check, Read and Write exist today, as the services `sungma-check`, `sungma-read` and `sungma-write`, over HTTP. Read reads by subjectset only. The [roadmap](roadmap.md) lists the rest.

## Inside a service

Each service is a set of bounded contexts over the table: names, write, Check and read. Each context has its own readers, writers and shapes, and shares only a small kernel of names, ids, the fact graph, rewrites and the epoch rule. The table knows nothing of what its items mean. It gets an item by key, queries a run of a partition, and commits a unit of work whole or not at all. One shared layout says where each kind of row lives, so every context reads the rows the others write. In tests, a table in memory stands in for DynamoDB.

## Where to look

- [Sample](sample.md): files, folders and groups, with a trace of one check.
- [Glossary](glossary): the domain terms.
- [Specs](specs): namespace documents, fact documents, namespace writes, the revision clock, one timeline and the table.
- [Notes](notes): open work and the current thinking on it.
