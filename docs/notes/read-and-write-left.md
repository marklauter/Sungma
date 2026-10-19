---
title: Read and write work left
type: note
summary: "The API reads and writes facts and namespaces beside POST /check; what remains is paging, reading by resource, a name cache for reads, and the order of a write's additions and retirements in the spec."
status: evolving
cites:
  - "[[fact]]"
  - "[[subjectset]]"
---

# Read and write work left

## Where it stands

The API checks access and reads and writes facts and namespaces, as JSON over HTTP, on DynamoDB or in memory. What follows is open.

## Open

1. **Reading by resource.** A read takes one [[subjectset]]. Reading every relation of one resource needs an index, since the table keys facts by subjectset.
2. **Paging.** A read gives every [[fact]] under the subjectset at once.
3. **Names in a large read.** A fact read looks up the names of each subject it finds one at a time, so a large subjectset repeats the same lookups. A lookup cache per read would cut them.
4. **Additions and retirements of one fact.** When a write both adds and retires a fact, the retirement applies first, so the fact is held after the write. [Fact documents](../specs/fact-documents.md#writes) doesn't say which comes first.

gRPC, audit of writes and opaque zookies are on the [roadmap](../roadmap.md).
