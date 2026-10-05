---
title: mark
type: definition
summary: "A token naming a snapshot and the staleness a read may accept."
status: evolving
names: "[[snapshot]]"
---

A token naming a snapshot and the staleness a read may accept.

## Rationale

Zanzibar's zookie and Kingo's kookie name the same concept. Pin fits only a decision, scope collides with OAuth scopes, and seal reads oddly on a request.
