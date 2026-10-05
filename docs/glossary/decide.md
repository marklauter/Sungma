---
title: decide
type: definition
summary: "To judge whether a subject is in a closure."
status: locked
evaluates: "[[closure]]"
queries:
  - "[[subjectset]]"
  - "[[subject]]"
issues: "[[decision]]"
---

To judge whether a subject is in a closure.

## Rationale

Replaces Kingo's contains. The same walk without grounds serves callers that need many verdicts and no audit. It is an optimization of deciding, not a separate term.
