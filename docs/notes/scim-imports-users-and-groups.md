---
title: SCIM imports users and groups
type: note
summary: "Sungma's SCIM v2 endpoint takes the users and groups its identity provider pushes, writes each group's members as facts under sungma.group, and refreshes the names and emails Author shows."
status: evolving
cites:
  - "[[rauthy-brokers-identity]]"
  - "[[sungma-authorization]]"
  - "[[identity]]"
  - "[[fact]]"
---

# SCIM imports users and groups

Sungma implements a SCIM v2 endpoint, and its identity provider pushes user and group changes to it, as [[rauthy-brokers-identity]] decides. The endpoint is the anti-corruption layer between SCIM's resources and Sungma's facts.

## What it writes

- **A group** becomes a resource in `sungma.group`, the namespace [[sungma-authorization]] defines for groups. Each member becomes a fact under `member`, such as `sungma.group:<group>#member@<sub>`, where `<sub>` is the member's [[identity]].
- **A user** refreshes the display projection: the name and email Author shows for a subject. Sungma keeps no user store beyond that, and the projection is never a source of truth.

A fact that names an imported group's members, such as `sungma.namespace:billing#reader@sungma.group:<group>#member`, then reaches each member, as it does for any group.

## Imported groups

- **SCIM is an ordinary caller.** The SCIM endpoint authenticates as a client of its own and writes through Write's policies, as [[sungma-authorization]] describes. It holds `group_creator`, so it can create groups. Creating a group writes an `owner` fact for its creator, so it owns each group it imports, and `member` is written by a group's owners, so it writes the members. The model decides all of this, with no special code.
- **The nesting rules apply.** A member fact that would form a cycle, or nest deeper or wider than the deployment's limits, is refused like any other. The import records each refusal, so it can be fixed in the identity provider.
- **An imported group is named by its SCIM `id`:** `scim/<id>`, such as `scim/8f3a9c`. Sungma imports from one source, its identity provider of record, so the `id` alone is unique. The `id` never changes, while the `displayName` can, so renaming a group in the identity provider changes nothing in Sungma. Author shows the display name. A group deleted and created again in the identity provider gets a new `id`, so it's a new group in Sungma too.
- **Sungma imports from the identity provider that issues its tokens,** so each member's id is the same `sub` that person's token carries. Systems upstream of it, such as an HR system feeding a directory, stay the source of truth. Sungma sits at the end of that chain.
- **Groups made in Author can't collide with imported ones.** No group named in Author may start with `scim/`.

## Deprovisioning

When the identity provider deletes a person or client, Sungma retires every identity fact whose subject is their `sub`: in customers' namespaces, in Sungma's own namespaces, and their group memberships. A `sub` the identity provider issues again then inherits nothing.

- Identity facts are stored under their subjectset, across every namespace. So each identity fact also gets an index item keyed by its subject, pointing to the fact, written and retired in the same write as the fact. Deprovisioning then reads one subject's index items and retires the facts they point to. The index items are ordinary items, so they work the same on every store and under the write-id protocol, unlike a DynamoDB secondary index.
- The deployment's owners are the exception. Their facts are written only by deployment setup, and a deployment keeps at least two. Deleting an owner in the identity provider doesn't retire their `owner` fact. It records that setup needs to run.

## Open

1. Whether an imported group can be edited in Sungma. The next SCIM push would overwrite a change made in Author, so imported groups may be read-only in Sungma, with their members changed only in the identity provider.
2. Whether the endpoint maps more than users and groups, such as custom attributes to relations.
