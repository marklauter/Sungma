---
title: Sungma's own authorization
type: specification
summary: "Sungma decides who may change and read its own namespaces and facts with four namespaces of its own: `sungma` for the deployment, `sungma.namespace` for each namespace's document, `sungma.relation` for the facts under each relation, and `sungma.team` for groups of subjects. Every subject, person or client, gets a role through facts like any other, directly or through a team."
status: evolving
cites:
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[fact]]"
  - "[[subject]]"
  - "[[identity]]"
  - "[[subjectset]]"
  - "[[resource]]"
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
---

# Sungma's own authorization

Sungma authorizes the callers of its own services with its own model. Who may create a [[namespace]], edit one, or write and read the [[fact]]s under a [[relation]] is itself a set of facts, checked like any other. Author, Sungma's UI, shows each person only what those facts allow.

## Subjects

A role is held by a [[subject]], and a subject is a person or a client alike. The subject of a request is its token's `sub` claim, an [[identity]], as [[check-takes-a-caller-token-and-a-subject-token]] decides. Sungma doesn't tell people and clients apart.

## Two tiers

The model separates two kinds of work:

- **Writing namespaces** is operator work. A namespace's rewrites decide who can reach what, so whoever edits one needs to understand the security it encodes.
- **Writing facts** is everyday work, such as sharing an invoice. Most people only ever write facts, and usually under a few relations.

Neither implies the other. A namespace's writer can't write its facts unless they're also granted that, and a fact writer can't write the namespace.

## The model

```toml
# The deployment: its admins, operators and auditors, and who may create namespaces
[sungma]
admin = { subjects = ["identity", "sungma.team#member"] }
namespace_creator = { rewrite = "this | admin", subjects = ["identity", "sungma.team#member"] }
operator = { rewrite = "this | admin", subjects = ["identity", "sungma.team#member"] }
auditor = { rewrite = "this | admin", subjects = ["identity", "sungma.team#member"] }

# Each namespace: who owns it, who writes its document, and who works with all its facts
["sungma.namespace"]
parent = { subjects = ["sungma"] }
owner = { rewrite = "this | (parent, admin)", subjects = ["identity", "sungma.team#member"] }
writer = { rewrite = "this | owner", subjects = ["identity", "sungma.team#member"] }
reader = { rewrite = "this | writer | fact_reader | (parent, auditor)", subjects = ["identity", "sungma.team#member"] }
fact_writer = { rewrite = "this | owner", subjects = ["identity", "sungma.team#member"] }
fact_reader = { rewrite = "this | fact_writer | (parent, auditor)", subjects = ["identity", "sungma.team#member"] }

# Each relation: who writes and reads the facts under it
["sungma.relation"]
parent = { subjects = ["sungma.namespace"] }
writer = { rewrite = "this | (parent, fact_writer)", subjects = ["identity", "sungma.team#member"] }
reader = { rewrite = "this | writer | (parent, fact_reader)", subjects = ["identity", "sungma.team#member"] }

# Each team: who owns it and who is in it
["sungma.team"]
parent = { subjects = ["sungma"] }
owner = { rewrite = "this | (parent, admin)", subjects = ["identity", "sungma.team#member"] }
member = { subjects = ["identity", "sungma.team#member"] }
```

Drawn as a graph, each namespace is a node, and each relation's accepted subjects are its edges. Solid edges are the `parent` links that rewrites follow. Dashed edges go to identities, the people and clients who hold roles, and to teams, which hold roles for their members.

```mermaid
flowchart LR
  relation["<b>sungma.relation</b><br/>parent<br/>writer<br/>reader"]
  namespace["<b>sungma.namespace</b><br/>parent<br/>owner<br/>writer<br/>reader<br/>fact_writer<br/>fact_reader"]
  sungma["<b>sungma</b><br/>admin<br/>namespace_creator<br/>operator<br/>auditor"]
  team["<b>sungma.team</b><br/>parent<br/>owner<br/>member"]
  identity(["identity"])
  relation -- parent --> namespace
  namespace -- parent --> sungma
  team -- parent --> sungma
  relation -. "writer, reader" .-> identity
  namespace -. "owner, writer, reader, fact_writer, fact_reader" .-> identity
  sungma -. "admin, namespace_creator, operator, auditor" .-> identity
  team -. "owner, member" .-> identity
  relation -. "every role" .-> team
  namespace -. "every role" .-> team
  sungma -. "every role" .-> team
  team -. "owner, member" .-> team
```

Facts name four kinds of [[resource]]:

- `sungma:deployment`, the one deployment. Setup's first fact names it.
- `sungma.namespace:<name>`, one for each namespace, such as `sungma.namespace:billing`.
- `sungma.relation:<namespace>/<relation>`, one for each relation, such as `sungma.relation:billing/approver`. A relation name holds no `/`, so the id reads one way.
- `sungma.team:<name>`, one for each team, such as `sungma.team:billing-ops`.

Two kinds of fact link these resources, and only Sungma writes them:

- `sungma.namespace:billing#parent@sungma:deployment#...` makes every admin an owner of `billing`.
- `sungma.relation:billing/approver#parent@sungma.namespace:billing#...` lets `billing`'s fact writers and fact readers reach its `approver` facts.

## What each role allows

| To do this | the subject needs |
|---|---|
| Grant or revoke `admin`, `namespace_creator`, `operator` or `auditor` | `admin` on `sungma:deployment` |
| Create a namespace | `namespace_creator` on `sungma:deployment` |
| Create a team | `admin` on `sungma:deployment` |
| Add or remove a team's members, or grant or revoke its `owner` | `owner` on the team |
| Run the deployment: read its logs and metrics | `operator` on `sungma:deployment` |
| Read and replay the audit records of decisions | `auditor` on `sungma:deployment` |
| Write a new version of a namespace, or retire it | `writer` on the namespace |
| Read a namespace's document | `reader` on the namespace |
| Grant or revoke a role on a namespace or one of its relations | `owner` on the namespace |
| Write facts under a relation | `writer` on the relation |
| Read facts under a relation, or check a [[subjectset]] formed from it | `reader` on the relation |

A fact's subject may name a resource or subjectset in another namespace. Writing the fact needs `writer` on its own relation only, nothing on the namespace it points into.

An auditor reads every namespace's document and every fact, and changes nothing.

A namespace-wide grant flows down: `fact_writer` on `billing` makes its holder a `writer` on every relation in `billing`, including relations added later.

## Protected namespaces

Write refuses any change to the `sungma`, `sungma.namespace`, `sungma.relation` and `sungma.team` namespaces themselves. Only a Sungma release changes them, since an edit could lock everyone out.

Write also refuses a fact under `parent` from any caller. Sungma writes those facts itself, as [Creating a namespace](#creating-a-namespace) and [New relations](#new-relations) say. A caller who could write one could tie a namespace to a resource they control.

## The first admin

Deployment setup names one subject and writes `sungma:deployment#admin@<subject>`. The identity provider is seeded with a user under that same id, so the first person to sign in to Author is already an admin. Nobody becomes admin by signing in first.

## The last admin

Revoking `admin` from the last subject that holds it is refused. The revocation reads the admins at the head it's based on, in its own unit of work. Every write commits only while the head is unchanged. When two admins revoke each other at once, one commits, and the other retries, finds one admin left, and is refused.

## Creating a namespace

Creating a namespace stores it, the fact tying it to the deployment, and an `owner` fact for its creator, in one unit of work. Either all three are stored or none is, so a namespace never exists without an owner.

## Teams

A team holds a role for its members. One fact, such as `sungma.namespace:billing#reader@sungma.team:billing-ops#member`, makes every member of `billing-ops` a reader of `billing`. A team's members can include other teams, so membership nests.

Creating a team stores its `parent` fact and an `owner` fact for its creator in one unit of work, as creating a namespace does. Admins own every team through `parent`.

## New relations

A namespace version that declares a relation the namespace's earlier versions didn't stores the relation's `parent` fact in the same unit of work. A relation a later version drops keeps its fact. The fact is then inert, like any fact under a dropped relation.

## Required tests

1. Each role in [What each role allows](#what-each-role-allows) allows its operation, and a subject without it is refused.
2. `fact_writer` on a namespace allows writing facts under every relation in it, including one a later version adds.
3. A namespace writer who isn't a fact writer can't write facts, and a fact writer who isn't a namespace writer can't write a namespace version.
4. Admins own every namespace without an `owner` fact of their own.
5. A person and a client with the same roles get the same answers.
6. Writes that change `sungma`, `sungma.namespace`, `sungma.relation` or `sungma.team`, and facts under `parent` from a caller, are refused.
7. Two admins revoking each other at once leave one admin.
8. A failed namespace creation stores none of its three items.
9. A new relation gets its `parent` fact in the same unit of work as the version that declares it.
10. An auditor reads every namespace's document and the facts under every relation, and every write by an auditor who holds no other role is refused.
11. A role granted to a team reaches its members, including members of teams nested inside it, and a team's owner can change its members without holding any other role.

## Open

1. How deployment setup runs, and where it takes the first admin's subject from.
2. What `operator` covers beyond logs and metrics, such as health, and whether operators may read namespace documents to diagnose a problem.
3. Whether a namespace's owner should be able to transfer ownership, or revoke the last owner while admins still own it through the deployment.
