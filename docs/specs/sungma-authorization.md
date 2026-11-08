---
title: Sungma's own authorization
type: specification
summary: "Sungma decides who may change and read its own namespaces and facts with four namespaces of its own: `sungma` for the deployment, `sungma.namespace` for each namespace's document, `sungma.relation` for the facts under each relation, and `sungma.group` for groups of subjects. Each endpoint runs one fixed check, and each relation's `written_by` and `read_by` name who may write and read its facts, so the model alone decides who passes."
status: evolving
cites:
  - "[[scim-imports-users-and-groups]]"
  - "[[privileged-roles-need-a-recent-sign-in]]"
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[fact]]"
  - "[[subject]]"
  - "[[identity]]"
  - "[[subjectset]]"
  - "[[resource]]"
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
  - "[[catalog]]"
  - "[[expand]]"
  - "[[epoch]]"
---

# Sungma's own authorization

Sungma authorizes the callers of its own services with its own model. Who may create a [[namespace]], edit one, or write and read the [[fact]]s under a [[relation]] is itself a set of facts, checked like any other. Author, Sungma's UI, shows each person only what those facts allow.

Every authorization rule is a check against the model. Each endpoint runs one fixed check, and the model decides who passes it. No code names a role or a protected fact.

## Subjects

A role is held by a [[subject]]: a person's or client's [[identity]], or a group's members. The caller of a request is its token's `sub` claim, an identity, as [[check-takes-a-caller-token-and-a-subject-token]] decides. For Check, that is the caller token's `sub`, not the subject the check is about. Sungma doesn't tell people and clients apart.

## Two tiers

The model separates two kinds of work:

- **Writing namespaces** is specialist work. A namespace's [[rewrite]]s decide who can reach what, so whoever edits one needs to understand the security it encodes.
- **Writing facts** is everyday work, such as sharing an invoice. Most people only ever write facts, and usually under a few relations.

Neither implies the other. A namespace's writer can't write its facts unless they're also granted that, and a fact writer can't write the namespace.

## The model

```toml
# The deployment: its owners and admins, operators and auditors, who may create namespaces and groups, and who reads those grants
[sungma]
owner = { subjects = ["identity"], written_by = "sungma", read_by = "grant_reader" }
admin = { rewrite = "this | owner", subjects = ["identity"], written_by = "owner", read_by = "grant_reader" }
namespace_creator = { rewrite = "this | admin", subjects = ["identity", "sungma.group#member"], written_by = "admin", read_by = "grant_reader" }
group_creator = { rewrite = "this | admin", subjects = ["identity", "sungma.group#member"], written_by = "admin", read_by = "grant_reader" }
operator = { rewrite = "this | admin", subjects = ["identity", "sungma.group#member"], written_by = "admin", read_by = "grant_reader" }
auditor = { rewrite = "this | admin", subjects = ["identity", "sungma.group#member"], written_by = "admin", read_by = "grant_reader" }
grant_reader = "auditor"

# Each namespace: who owns it, who writes its document, who works with all its facts, and who reads those grants
["sungma.namespace"]
parent = { subjects = ["sungma"], written_by = "sungma", read_by = "grant_reader" }
protected = { subjects = ["sungma"], written_by = "sungma", read_by = "grant_reader" }
owner = { rewrite = "this | (parent, admin) ! (protected, admin)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
writer = { rewrite = "this | owner", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
reader = { rewrite = "this | writer | fact_reader | (parent, auditor)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
fact_writer = { rewrite = "this | owner", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
fact_reader = { rewrite = "this | fact_writer | (parent, auditor)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
grant_reader = "owner | (parent, auditor)"

# Each relation: who owns it through its namespace, who writes and reads the facts under it, and who reads those grants
["sungma.relation"]
parent = { subjects = ["sungma.namespace"], written_by = "sungma", read_by = "grant_reader" }
owner = "(parent, owner)"
writer = { rewrite = "this | (parent, fact_writer)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
reader = { rewrite = "this | writer | (parent, fact_reader)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
grant_reader = "(parent, grant_reader)"

# Each group: who owns it, who is in it, and who reads those grants
["sungma.group"]
parent = { subjects = ["sungma"], written_by = "sungma", read_by = "grant_reader" }
owner = { rewrite = "this | (parent, admin)", subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
member = { subjects = ["identity", "sungma.group#member"], written_by = "owner", read_by = "grant_reader" }
grant_reader = "owner | (parent, auditor)"
```

Drawn as a graph, each namespace is a node, and each relation's accepted subjects are its edges. Solid edges are the `parent` and `protected` links that rewrites follow. Dashed edges go to identities, the keys of the people and clients who hold roles, and to groups, which hold roles for their members.

```mermaid
flowchart LR
  relation["<b>sungma.relation</b><br/>parent<br/>owner<br/>writer<br/>reader<br/>grant_reader"]
  namespace["<b>sungma.namespace</b><br/>parent<br/>protected<br/>owner<br/>writer<br/>reader<br/>fact_writer<br/>fact_reader<br/>grant_reader"]
  sungma["<b>sungma</b><br/>owner<br/>admin<br/>namespace_creator<br/>group_creator<br/>operator<br/>auditor<br/>grant_reader"]
  group["<b>sungma.group</b><br/>parent<br/>owner<br/>member<br/>grant_reader"]
  identity(["identity"])
  relation -- parent --> namespace
  namespace -- parent --> sungma
  namespace -- protected --> sungma
  group -- parent --> sungma
  relation -. "writer, reader" .-> identity
  namespace -. "owner, writer, reader, fact_writer, fact_reader" .-> identity
  sungma -. "owner, admin, namespace_creator, group_creator, operator, auditor" .-> identity
  group -. "owner, member" .-> identity
  relation -. "every role" .-> group
  namespace -. "every role" .-> group
  sungma -. "namespace_creator, group_creator, operator, auditor" .-> group
  group -. "owner, member" .-> group
```

`sungma.relation`'s `owner` accepts no facts. It reads the owners of the relation's namespace through `parent`. Each `grant_reader` accepts no facts either. It names who reads the grants on a resource, as [Who reads each relation](#who-reads-each-relation) says.

In `sungma.namespace`'s `owner`, `!` binds tighter than `|`, so the rewrite reads `this | ((parent, admin) ! (protected, admin))`. The admins reached through `parent` are owners, except on a namespace that `protected` ties to the deployment. [Protected namespaces](#protected-namespaces) says why.

Facts name four kinds of [[resource]]:

- `sungma:deployment`, the one deployment. Setup's first fact names it.
- `sungma.namespace:<name>`, one for each namespace, such as `sungma.namespace:billing`.
- `sungma.relation:<namespace>/<relation>`, one for each relation, such as `sungma.relation:billing/approver`. A relation name holds no `/`, so the id reads one way.
- `sungma.group:<name>`, one for each group, such as `sungma.group:billing-ops`.

Four kinds of fact link these resources, three under `parent` and one under `protected`:

- `sungma.namespace:billing#parent@sungma:deployment#...` makes every admin an owner of `billing`, and every auditor a reader of it.
- `sungma.relation:billing/approver#parent@sungma.namespace:billing#...` lets `billing`'s owners, fact writers and fact readers reach its `approver` facts.
- `sungma.group:billing-ops#parent@sungma:deployment#...` makes every admin an owner of `billing-ops`.
- `sungma.namespace:sungma.group#protected@sungma:deployment#...` takes the admins back out of the owners of `sungma.group`. Setup writes one for each of Sungma's own namespaces.

## Who writes each relation

A relation may declare which relation on the same resource may write its facts, as `written_by = "<relation>"`. [Namespace documents](namespace-documents.md#accepted-subjects) gives its form.

Writing a fact `N:r#R@s`, either adding it or retiring it, checks `N:r#W` for the caller, where `W` is `R`'s `written_by`. The check stays on the fact's own resource. A grant on `sungma.namespace:billing#reader` checks `sungma.namespace:billing#owner`, so an owner of `billing` grants `reader` on `billing` and on no other namespace.

A relation that declares no `written_by` is written by whoever holds `sungma.relation:N/R#writer`. That is how most relations in a customer's namespace are written. Every relation in Sungma's own namespaces that accepts facts declares `written_by`, so none of them falls back to a resource shared by every namespace. This is a relation's write policy.

`written_by = "sungma"` means no caller writes the relation through the API. Only Sungma's own tooling writes it: deployment setup, run out of band with the deployment's credentials, and Sungma releases. Setup is also the break-glass path, for example to replace a lost or untrustworthy deployment owner.

A namespace's `written_by` options are part of the security it encodes, so declaring them is the namespace writer's work.

Each protection in the model rests on one of these declarations:

- `sungma:deployment#owner` is `written_by = "sungma"`. No caller grants or revokes `owner`, an owner included, so no one takes the deployment over through the API, and no one locks its owners out.
- `sungma:deployment#admin` is `written_by = "owner"`. Admins run everything else but can't grant or revoke `admin`, so no admin can remove another.
- `parent` in `sungma.namespace`, `sungma.relation` and `sungma.group`, and `protected` in `sungma.namespace`, are `written_by = "sungma"`. Sungma writes those facts itself, as [Creating a namespace](#creating-a-namespace), [New relations](#new-relations) and [Groups](#groups) say. A caller who could add one could tie a namespace to a resource they control. A caller who could retire one could cut a namespace, relation or group off from the admins and auditors who reach it through `parent`. A caller who could retire a `protected` fact could make the admins owners of one of Sungma's own namespaces.
- A group's `member` is `written_by = "owner"`, so a member can't leave a group on their own.

## Who reads each relation

A relation may declare which relation on the same resource may read its facts, as `read_by = "<relation>"`. Its holders read the facts under the relation and check [[subjectset]]s formed from it. [Namespace documents](namespace-documents.md#accepted-subjects) gives its form.

Reading the facts under `N:r#R`, or checking `N:r#R`, checks `N:r#B` for the caller, where `B` is `R`'s `read_by`. Like a write, the check stays on the fact's own resource. A grant on `sungma.namespace:billing#reader` is read through `sungma.namespace:billing#grant_reader`, so `billing`'s owners and the deployment's auditors read it, and an owner of another namespace doesn't.

A relation that declares no `read_by` is read by whoever holds `sungma.relation:N/R#reader`. Reads don't follow writes: a relation that declares `written_by` and no `read_by` is read through `sungma.relation:N/R#reader`, not through its writers. This is a relation's read policy.

Every relation in Sungma's own namespaces that accepts facts declares `read_by = "grant_reader"`. Each namespace's `grant_reader` names who reads its grants:

- On `sungma`, the auditors. Every admin is an auditor, so admins and owners read the deployment's grants too.
- On `sungma.namespace`, the namespace's owners and the deployment's auditors.
- On `sungma.relation`, whoever reads the grants on the relation's namespace, through `parent`.
- On `sungma.group`, the group's owners and the deployment's auditors.

A relation whose rewrite doesn't read `this` accepts no facts, so it declares neither `written_by` nor `read_by`. A check on a subjectset formed from it, such as `sungma.namespace:billing#grant_reader`, falls back to `sungma.relation:N/R#reader`. In Sungma's own namespaces, that reaches the auditors through the relation's `parent`, which every version stores for a relation it adds, as [New relations](#new-relations) says, a Sungma release's versions included.

`read_by` has no `sungma` keyword. A relation nobody may read would leave its facts out of every audit.

## Policies

Each endpoint has a policy, a fixed check it runs. An endpoint's policy is fixed, and the model decides who passes it. So adding a role, or changing who may write or read a relation, is a model change, never a code change. No code holds a list of protected facts or roles.

A relation's write policy and read policy are the checks the two sections above give:

- `R`'s write policy on `N:r` is `N:r#W` when `R` declares `written_by = "W"`, refused for every caller when it declares `written_by = "sungma"`, and `sungma.relation:N/R#writer` when it declares no `written_by`.
- `R`'s read policy on `N:r` is `N:r#B` when `R` declares `read_by = "B"`, and `sungma.relation:N/R#reader` when it declares no `read_by`.

Every check runs for the caller, the caller token's `sub`.

| Service | Endpoint | Policy (the check it runs) |
|---|---|---|
| Read | Read a fact by its key, `N:r#R@s` | `R`'s read policy on `N:r` |
| Read | Read the facts under a subjectset `N:r#R` | `R`'s read policy on `N:r` |
| Read | Read the facts of a resource `N:r` | each relation's read policy on `N:r`; the read holds only the relations whose policy the caller passes |
| Write | Write facts: each fact under `N:r#R`, added or retired | `R`'s write policy on `N:r` |
| Write | Create namespace `N` | `sungma:deployment#namespace_creator` |
| Write | Write a new version of namespace `N` that declares every relation the latest version declares | `sungma.namespace:N#writer` |
| Write | Write a new version of namespace `N` that drops a relation the latest version declares | `sungma.namespace:N#owner` |
| Write | Retire namespace `N` | `sungma.namespace:N#owner` |
| Write | Create group `G` | `sungma:deployment#group_creator` |
| Write | Change a deployment setting | `sungma:deployment#admin` |
| Check | Check whether a subject is in `N:r#R` | `R`'s read policy on `N:r` |
| Expand | [[expand]] `N:r#R` | `R`'s read policy on `N:r`; see [open question 11](#open) |
| Watch | Watch the changelog stream | open; see [open question 12](#open) |
| Namespaces | Read namespace `N`'s document at a revision | `sungma.namespace:N#reader` |
| Namespaces | List namespace `N`'s versions | `sungma.namespace:N#reader` |
| Namespaces | List the [[catalog]] | `sungma.namespace:N#reader` for each namespace `N`; the list holds only the namespaces whose policy the caller passes |
| Operations | Read logs and metrics | `sungma:deployment#operator` |
| Operations | Read and replay the audit records of decisions | `sungma:deployment#auditor` |

The fact-write endpoint applies its policy to each fact in a batch, and refuses the batch if any fact fails.

A version that drops a relation needs `owner`, like retiring, because it has the same effect on that relation's facts: a later version that brings the relation back gives it a new [[epoch]], so every fact under it from before the drop stays dead. The policy looks only at whether the version drops a relation the latest version declares, never at which one. Adding relations, and changing the rewrites of the ones a version keeps, stay with `writer`.

A fact's subject may name a resource or subjectset in another namespace. Writing the fact checks only its own relation's policy, nothing on the namespace it points into.

## What follows from the model

This summary follows from the model and the policies. It isn't a separate rule.

- The deployment's owners decide the admins. Every owner is an admin.
- Admins grant `namespace_creator`, `group_creator`, `operator` and `auditor`, and own every namespace and group through `parent`.
- A namespace's owners grant every role on the namespace and its relations, read those grants, write new versions, drop relations and retire it. Its writers write versions that keep every relation.
- A namespace-wide grant flows down: `fact_writer` on `billing` makes its holder a `writer` on every relation in `billing`, including relations added later.
- A group's owners change its members and its owners, and read them.
- An auditor reads every namespace's document, Sungma's own included, every grant in Sungma's own namespaces, and the facts under every relation that declares no `read_by`, and changes nothing.
- A relation that declares `read_by` decides its readers alone. An auditor reads its facts only if that relation reaches the auditors.

## Protected namespaces

No caller changes the `sungma`, `sungma.namespace`, `sungma.relation` and `sungma.group` namespaces themselves. Only a Sungma release changes them, since an edit could lock everyone out. The model enforces this.

Setup writes a `parent` fact for each of their `sungma.namespace` resources, such as `sungma.namespace:sungma.group#parent@sungma:deployment#...`, so their documents are read by the same policy as any namespace document. Auditors read them through `reader`'s `(parent, auditor)`, and so do admins, who are auditors.

That `parent` link alone would make every admin an owner of Sungma's own namespaces through `(parent, admin)`, and so a writer of them, able to write a version or retire one. Setup therefore also writes a `protected` fact for each of them, such as `sungma.namespace:sungma.group#protected@sungma:deployment#...`. `owner`'s rewrite takes the admins reached through `protected` back out of those reached through `parent`, so Sungma's own namespaces have no owner. No caller can add an `owner` fact there, because `owner` is `written_by = "owner"`, which nobody holds there. With no owner, nobody holds `writer` or `fact_writer` either. Writing a version of `sungma.group` checks `sungma.namespace:sungma.group#writer` or `#owner`, and retiring it checks `#owner`, so each is refused for every caller. No caller can add or retire a `protected` fact, because `protected` is `written_by = "sungma"`.

The protection lives in the model rather than in a check on the reserved name. That way the model alone says who may write each namespace, and the policy for writing a version stays the same for every namespace. `protected` only takes admins out of `owner`, so a caller would gain nothing by protecting a namespace of their own, and can't anyway.

Sungma's own namespaces stay protected only while no namespace other than Sungma's own has `sungma` as its first segment, as [Namespace documents](namespace-documents.md#names) reserves. Creating one is refused, so a namespace such as `sungma.audit` can't pass for one of Sungma's own.

## The deployment's owners

A role is trust placed in the people who hold it. The deployment's owners are the people the organization trusts most: they decide who the admins are. Every owner is an admin too.

`owner` is `written_by = "sungma"`, so only deployment setup writes `owner` facts, run out of band by whoever controls the deployment's configuration. Setup names each owner by identity, the `sub` the identity provider issues to them, and each run is recorded for auditors. The first owners can act as soon as they first sign in to Author, and nobody becomes an owner by signing in first. Owners are admins, and nothing in the API removes them, so the deployment always has an admin.

A deployment has at least two owners. Setup refuses to leave fewer, so losing one owner's identity never leaves the deployment without one. Setup is also how the organization recovers from a lost or untrustworthy owner: it runs setup again with the owners it trusts. That doesn't guard against owners who abuse their trust. Nothing in the model can. Choosing them well is the organization's part.

## Authorizing a write

A write is authorized at the [head](table.md#the-head) it's based on, in its own [unit of work](table.md#the-table-and-the-contexts). Every write commits only while the head is unchanged, so a write that retries is authorized again at the new head. A role revoked by a write that committed first can't authorize a write that commits after it.

A read is authorized at the snapshot it reads, so a role revoked before that snapshot can't read data written after it.

## Creating a namespace

Creating a namespace stores it, the fact tying it to the deployment, an `owner` fact for its creator, and a `parent` fact for each of its relations, in one unit of work. Either all of them are stored or none is, so a namespace never exists without an owner.

## Groups

A group holds a role for its members. One fact, such as `sungma.namespace:billing#reader@sungma.group:billing-ops#member`, makes every member of `billing-ops` a reader of `billing`. A group's members can include other groups, so membership nests, within the limits [Checks on the data](#checks-on-the-data) sets.

Creating a group stores its `parent` fact and an `owner` fact for its creator in one unit of work, as creating a namespace does. Admins own every group through `parent`.

## New relations

A namespace version that declares a relation the namespace's earlier versions didn't stores the relation's `parent` fact in the same unit of work. A relation a later version drops keeps its `parent` fact and the grants on it. Nothing reads them while the relation is gone. A version that declares the relation again writes its `parent` fact afresh, opening a new span, so the relation keeps its link whatever rule [open question 6](#open) settles for grants.

## Checks on the data

Some rules aren't about who may write, but about the shape of the data. Write checks them separately from authorization, for every caller:

- Adding a fact under `sungma.relation:<namespace>/<relation>` is refused unless the namespace's latest version declares that relation. Otherwise a grant on a relation that doesn't exist yet would lie hidden, then take effect the day a version declares it. Retiring one is always allowed, so a grant on a dropped relation can still be revoked before the relation returns.
- Adding a fact whose subject is `sungma.group:<name>#member` is refused unless that group has been created. Otherwise a grant to a group that doesn't exist yet would take effect the day someone creates it.
- Creating a namespace whose first segment is `sungma` is refused, as [Protected namespaces](#protected-namespaces) says.
- Nesting is bounded, so a check that reaches a group never has to walk an unbounded graph.

The nesting limits are options on the `member` relation, declared in the model and generic to any relation. Write refuses a fact that makes a group a member of another group when:

- it would form a cycle, because the containing group is already reachable from the one being added;
- the longest chain of nested groups through it would be deeper than the depth limit;
- the containing group would hold more groups as direct members than the width limit. Identities don't count toward it, so a group imported from an identity provider can hold any number of people.

The two limits are deployment settings. They default to 10 levels deep and 32 groups wide. Write checks them at the head the write is based on, and checks them again when the write retries, so two writes can't each add half of a cycle.

## Conditions

Some rules need the request's context, which the model doesn't hold. Privileged writes need a recent sign-in, as [[privileged-roles-need-a-recent-sign-in]] says, and that needs the caller token's `auth_time`. Conditions on the roadmap enforce it: a privileged relation's condition requires a recent `auth_time`. Until conditions exist, how Sungma enforces it is [open](#open).

## Required tests

1. Each endpoint's policy allows a caller who passes its check, and refuses a caller who doesn't.
2. `fact_writer` on a namespace allows writing facts under every relation in it, including one a later version adds.
3. A namespace writer who isn't a fact writer can't write facts, and a fact writer who isn't a namespace writer can't write a namespace version.
4. Admins own every namespace without an `owner` fact of their own.
5. A person and a client with the same roles get the same answers.
6. A version of `sungma`, `sungma.namespace`, `sungma.relation` or `sungma.group`, and retiring one, are refused for every caller, an admin and a deployment owner included, because `protected` takes the admins out of their owners.
7. An admin who isn't an owner can't grant or revoke `admin`, because `admin` is `written_by = "owner"`.
8. A fact under `sungma:deployment#owner` is refused for every caller, an admin and an owner included, because `owner` is `written_by = "sungma"`.
9. A failed namespace creation stores none of its items.
10. A new relation gets its `parent` fact in the same unit of work as the version that declares it.
11. An auditor reads every namespace's document, the facts under every relation that declares no `read_by` and the grants under Sungma's own namespaces, and every write by an auditor who holds no other role is refused.
12. A role granted to a group reaches its members, including members of groups nested inside it, and a group's owner can change its members without holding any other role.
13. A write that retries is authorized again at the new head.
14. A failed group creation stores neither fact, and an admin owns every group without an `owner` fact.
15. A fact whose subject names another namespace needs only its own relation's policy.
16. A group's members can't be made admins through the group, because `admin` accepts identities only.
17. Adding or retiring a `parent` fact in `sungma.namespace`, `sungma.relation` or `sungma.group`, or a `protected` fact in `sungma.namespace`, is refused from every caller, an admin included, because both are `written_by = "sungma"`.
18. A grant on `sungma.relation:<namespace>/<relation>` is refused while the namespace doesn't declare the relation.
19. Creating a namespace whose first segment is `sungma` is refused.
20. Making a group a member of a group that it already contains, directly or through nesting, is refused, and so is a group made a member of itself.
21. Nesting groups one level deeper than the depth limit, or giving a group one more nested group than the width limit, is refused, and the limits come from the deployment's settings.
22. Two writes that would together form a cycle can't both commit.
23. Revoking a grant on a dropped relation is allowed, and a grant on a relation the namespace doesn't declare can't be added.
24. A relation declared again has a `parent` fact whose span opens at the new version.
25. A grant to a group that hasn't been created is refused.
26. An owner reads the grants on their namespace and its relations, and the members and owners of their group, and a caller without `owner` can't.
27. A member removing themselves from a group is refused unless they own it, because `member` is `written_by = "owner"`.
28. Changing a deployment setting needs `admin`.
29. A read at a snapshot taken after a role was revoked is refused for the caller who lost it.
30. Setup refuses to leave the deployment with fewer than two owners.
31. An owner of one namespace can grant `reader` on it, and can't grant `reader` on another namespace.
32. A grant on `sungma.relation:billing/approver#writer` checks `billing`'s owners: an owner of `billing` may add or retire it, and an owner of another namespace can't.
33. A `group_creator` who isn't an admin can create a group and owns it.
34. In a customer's namespace, a fact under a relation that declares `written_by` checks that relation on the fact's resource, and a fact under a relation that declares none checks `sungma.relation:N/R#writer`.
35. A batch of facts in which one fact fails its policy is refused whole.
36. An auditor reads the grants on any namespace, any relation and any group through `read_by`, and checks subjectsets formed from them.
37. A namespace's owner reads the grants on it and its relations, and an owner of another namespace can't.
38. An auditor reads the documents of `sungma`, `sungma.namespace`, `sungma.relation` and `sungma.group`, and no caller writes a version of them, an admin included.
39. Admins and auditors read the grants on `sungma:deployment`, and a `namespace_creator` who is neither can't.
40. In a customer's namespace, reading or checking a relation that declares `read_by` checks that relation on the fact's resource, and a relation that declares none checks `sungma.relation:N/R#reader`.
41. A relation that declares `written_by` and no `read_by` is read through `sungma.relation:N/R#reader`, so a caller who may write its facts and holds no reader role can't read them.
42. A read of a resource's facts holds only the relations whose read policy the caller passes.
43. Listing the catalog holds only the namespaces the caller reads.
44. A writer who isn't an owner can't write a version that drops a relation the latest version declares, and an owner can. A writer can write a version that adds a relation or changes a rewrite.

## Open

1. How deployment setup runs, where it takes the owners' identities from, and how its runs are recorded for auditors.
2. What `operator` covers beyond logs and metrics, such as health, and whether operators may read namespace documents to diagnose a problem.
3. Whether a namespace's owner should be able to transfer ownership, or revoke the last owner while admins still own it through the deployment.
4. How groups imported through SCIM are named, created and edited. See [[scim-imports-users-and-groups]].
5. How a namespace version that adds more relations than one unit of work holds gets its `parent` facts, and how namespace writes and the table make room for fact items in a namespace write. [Table](table.md#open) holds the general question of writes larger than one unit of work.
6. Whether grants start fresh when a relation comes back, or a retired namespace is created again. A relation brought back gets a new epoch, so its facts from before don't count. The grants on it, such as `sungma.relation:billing/approver#writer@alice`, sit under `sungma.relation`'s `writer`, which is never dropped, so they count again. A grant could count only if it was written after its relation's current epoch, the same rule its facts follow.
7. Who may create a retired namespace again. As written, anyone with `namespace_creator` can recreate a retired name and become its owner. Facts under the old namespace stay dead, since every relation gets a new epoch. But facts in other namespaces that point into it, such as `file:secret#viewer@billing:acct-42#member`, live under their own namespace and still count, and the new owner now decides who is in `billing:acct-42#member`. Reusing a retired name stays possible, since that's what namespace versions are for. The options are about who may do it:
   - Recreating a retired name needs `owner` on it. The old owners' grants survive retirement, because they live in `sungma.namespace`, and admins reach it through `parent`.
   - Only admins may recreate a retired name, and they grant ownership afresh. This pairs with old grants ending at the new epoch, as question 6 proposes.
   - A proposed answer: a fact whose subject is in another namespace records that subject's epoch when it's written. That's the epoch of the subject's relation, such as `billing`'s `member`, or for a resource member such as `folder:root#...`, the epoch the subject's namespace was last created at. The fact counts only while both its own relation's epoch and its recorded subject epoch still hold. Recreating `billing` gives `member` a new epoch, so `file:secret#viewer@billing:acct-42#member` stops counting without being touched, and a recreated namespace inherits nothing that pointed into it. A namespace recreated under its old name is the same namespace, started fresh. Reads at earlier revisions still show its old versions, relations and facts. A fact pointing into it that is wanted again is written again, and records the new epoch. This changes [One timeline](one-timeline.md), which says when a fact counts, and [Table](table.md), which says what a span stores.
8. How a relation's options declare the nesting limits, no cycles and bounded depth and width, so they're declared on `member` in the model and apply to any relation that declares them.
9. Whether the other checks on the data, a grant only on a declared relation and a grant only to a created group, become relation options too, so no check on the data names one of Sungma's namespaces.
10. How privileged writes need a recent sign-in until conditions exist. Once they do, a privileged relation's condition requires a recent `auth_time`, as [Conditions](#conditions) says.
11. Whether Expand checks the read policy of each subjectset it reaches, as well as the one it expands. An expansion shows the members of subjectsets under other relations, such as a group's members, that the caller may not be able to read on their own.
12. Which policy Watch runs. The changelog stream carries facts under many relations, and namespace versions across the catalog, so no check on one resource covers it. Filtering each change by its relation's read policy at its revision stays in the model, but it isn't one fixed check, and it leaves open whether a caller learns that a change it can't read happened. A deployment relation, such as a `watcher` on `sungma`, would be one fixed check, but its holders would read every fact.
13. Whether a customer's relation that declares `read_by` should reach the deployment's auditors. As written, its own model decides, and an auditor reads its facts only if it reaches them.
