---
title: subject
type: definition
summary: "An identity, a subjectset, or a resource bound to a subjectset."
status: locked
may-be:
  - "[[identity]]"
  - "[[subjectset]]"
  - "[[resource]]"
bound-to: "[[subjectset]]"
---

An identity, a subjectset, or a resource bound to a subjectset.

## Examples

- `carol`
- `group:eng#member`
- `folder:root#...`, a resource member

## Rationale

A resource in the subject position is called a resource member and written `theory:id#...`. Parent links are stored this way, and a fact-to-subjectset follows one to the resource it names. Resource member names that case and has no entry of its own: defining it as a kind of subject would make this entry circular.
