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

## Deprovisioning

When the identity provider deletes a person or client, Sungma retires every identity fact whose subject is their `sub`: in customers' namespaces, in Sungma's own namespaces, and their group memberships. A `sub` the identity provider issues again then inherits nothing.

- Identity facts are stored under their subjectset, across every namespace, so finding all the facts about one `sub` needs an index by subject. Until there is one, deprovisioning runs as a background job that reads the whole table.
- The deployment's owners are the exception. Their facts are written only by deployment setup, and a deployment keeps at least two. Deleting an owner in the identity provider doesn't retire their `owner` fact. It records that setup needs to run.

## Open

1. How groups are named. A SCIM group's `id` never changes and its `displayName` can, so a resource id taken from the display name would orphan every fact that names the group when it's renamed.
2. Who creates an imported group and writes its members. The SCIM endpoint would write the group's `parent` and `member` facts, so creating a group wouldn't be for admins alone.
3. Whether an imported group can be edited in Sungma. The next SCIM push would overwrite a change made in Author, so imported groups may be read-only in Sungma, with their members changed only in the identity provider.
4. Whether the endpoint maps more than users and groups, such as custom attributes to relations.
