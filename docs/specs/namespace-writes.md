---
title: Namespace writes
type: specification
summary: "A namespace write stores a whole new version of one namespace, or retires it, and is refused unless the namespace is still at the version the write was based on. No namespace write scans facts."
status: evolving
cites:
  - "[[namespace]]"
  - "[[relation]]"
  - "[[revision]]"
  - "[[revision-token]]"
  - "[[span]]"
  - "[[epoch]]"
  - "[[orphaned-fact]]"
  - "[[catalog]]"
  - "[[fact]]"
  - "[[revision-clock]]"
---

# Namespace writes

A namespace write changes one [[namespace]] in the [[catalog]] at a [[revision]]. It never edits a stored version. It stores a new version, closes the latest version's [[span]], or both. [One timeline](one-timeline.md) says what a read at any revision sees, and [Namespace documents](namespace-documents.md) says what a version may contain.

## Writes

There are three:

- **Create** stores the first version of a namespace that has no latest version. Creating a namespace that has one is refused.
- **Write a new version** stores the next version of a namespace that has a latest version. The latest version's span closes at the new version's revision.
- **Retire** closes the latest version's span and stores no new version. From then on the namespace declares no relations, so every fact under it is an [[orphaned-fact]]. A retired namespace can be created again.

Each version keeps an [[epoch]] for each [[relation]] it declares. A relation that carries over to a new version keeps its epoch. A relation the new version adds or brings back gets the new version's revision as its epoch. Creating a namespace again after it was retired brings every relation back. [One timeline](one-timeline.md#retiring-a-namespace) walks through a sample.

## What a write carries

A create or a new version carries one whole namespace document. The document becomes the whole version, so `Namespace::new` checks it whole. A write carrying only some relations would have to be merged into the latest version before it could be checked. A document that fails any check is refused, and nothing is stored.

A namespace may declare no relations. Its resources are then only ever subjects of facts in other namespaces.

## Concurrent writes

A write names the version it was based on, like an etag:

- Writing a new version, or retiring, carries the [[revision-token]] of the version the caller read.
- Creating carries none, since there is no version to have read.

The write is refused when the namespace's latest version isn't the one named. The caller then reads the latest version and tries again. Two writers that read the same version can't both succeed, so neither silently undoes the other.

The store meets this with a conditional write on the namespace, the same primitive the interner uses for `insert_if_absent`. A store without one can't host Sungma, since the interner needs it too.

## Facts

A namespace write never reads or rewrites facts. It doesn't report the facts it orphans, and it doesn't refuse past some number of them. Either would need a scan, and with billions of facts a scan would make every namespace write as slow as the graph is large. Which facts count after a write follows from spans and epochs, as [One timeline](one-timeline.md) specifies. Readers apply that rule when they read. The write only moves epochs.

Dropping a relation empties it everywhere, including in other namespaces that reach it through a fact-to-subjectset. On the excluded side of an exclusion, that widens access. [One timeline](one-timeline.md#what-is-accepted) accepts it.

## Storage

- A version is stored as the printer's form of the checked namespace, never as the text the caller sent. The printer sorts relations by name, so a version prints the same document every time. It's stored as that printed document, with names, and a read parses it and maps the names to ids. A name keeps its id for good, so the document reads back the same version.
- Namespace and relation names are interned when a namespace is written, and only then. Checks look names up and never intern them. See [Interner](../interner.md).
- A namespace write takes its revision from the [[revision-clock]] and waits out commit-wait before acknowledging, as a fact write does. See [Clock](clock.md).
- A namespace write names the version it expects. A namespace read finds the version whose span covers a revision.

## Answered elsewhere

- **The response,** such as whether a write returns the new version's revision token and printed document: the transports, roadmap item 14.
- **Reading namespaces,** a namespace at a revision token and a list of its versions: the transports, roadmap item 14.
- **Errors,** the status and body for each refusal: the transports, roadmap item 14.
- **Propagation,** how a new version reaches other nodes and appears in the Watch stream: the Watch service, roadmap item 11. Namespace versions go into the same change log as facts.
- **Who may write:** authentication and authorization, roadmap item 15. See [[check-takes-a-caller-token-and-a-subject-token]].

## Required tests

1. Creating a namespace that has a latest version is refused, and creating one that was retired succeeds.
2. A new version based on the latest version is stored, its span opens at its revision, and the previous version's span closes there.
3. A new version or a retirement based on an older version is refused, and nothing is stored.
4. Of two writes based on the same version, exactly one succeeds.
5. A read at a revision between two versions sees the earlier one.
6. A relation that carries over to a new version keeps its epoch. A relation one version drops and a later version brings back gets the later version's revision as its epoch.
7. After a retirement, every fact under the namespace counts as nonexistent, and a replay at a revision before the retirement still sees those facts.
8. After the namespace is created again, facts from before its retirement still count as nonexistent, and a fact written again after it counts.
9. A document that fails any check stores nothing.
10. A namespace write reads no fact. A store that fails every fact read doesn't fail the write.
