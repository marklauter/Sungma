---
title: Reusing a resource id
type: note
summary: "A resource id that names something deleted and created again brings back every fact about the old one. Back resources with ids that never repeat, or accept that recreating something restores its facts."
status: evolving
cites:
  - "[[resource]]"
  - "[[fact]]"
  - "[[sungma-authorization]]"
---

# Reusing a resource id

A [[resource]] id is opaque to Sungma. Sungma doesn't know when the thing an id stands for is deleted, or when another thing gets the same id. So a [[fact]] about `project:apollo` applies to whatever `apollo` is now.

Say Bob owns a project named Apollo, stored as `project:apollo#owner@bob`. Apollo is deleted, and months later someone creates a new project named Apollo. Bob's fact is still there, so Bob owns the new Apollo.

Only the code that creates and deletes the thing knows when an id is reused, so that code chooses one of two ways:

1. **Back each resource with an id that never repeats,** such as a database key: `project:7f3a9c` instead of `project:apollo`. A thing created again gets a new id, and no earlier fact mentions it. Display the name, and store the id.
2. **Use the name as the id, and accept that creating it again restores its facts.** That's the right choice where restoring is what people want, such as bringing back a project deleted by accident with all its access intact.

Code that takes the first way can still retire a deleted resource's facts, to keep them from building up.

Sungma takes the first way for its own namespaces, relations and groups. Each creation has its own id, as [[sungma-authorization]] describes under names and creations.
