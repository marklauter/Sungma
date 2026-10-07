---
title: revision clock
type: definition
summary: "An ordering of writes for measuring spans."
status: evolving
---

An ordering of writes for measuring spans.

## Examples

- A theory version written at revision 3 and replaced at revision 7 holds over [3, 7) on the revision clock.

## Rationale

A revision can be a timestamp or a number that only increases. Either way it is a reading of the revision clock, and spans, snapshots and catalogs are all measured against it. The wall clock plays no part.
