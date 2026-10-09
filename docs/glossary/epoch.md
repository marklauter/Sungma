---
title: epoch
type: definition
summary: "The revision of the theory version that last introduced a relation, before which no fact under the relation counts."
status: evolving
is-a: "[[revision]]"
---

The revision of the theory version that last introduced a relation, before which no fact under the relation counts.

## Examples

- `file` v1 at revision 3 declares `viewer`, so its epoch is 3, and `file:x#viewer@carol` is written at 5. Version v2 at 7 keeps `viewer`, so its epoch stays 3 and carol's fact counts.
- Version v3 at 10 drops `viewer`, and v4 at 12 declares it again with epoch 12. In the snapshot at 13, carol's fact opened at 5, before 12, so it doesn't count and the dropped grant doesn't come back.

## Rationale

A snapshot picks a theory version by revision, and the epoch then filters the facts under each relation that version declares. A fact counts when its [[span]] covers the snapshot's revision and opened at or after its relation's epoch. Each theory version records an epoch per relation, so a replay at an earlier revision reads the earlier epoch.
