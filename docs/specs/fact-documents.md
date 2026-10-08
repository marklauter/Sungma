---
title: Fact documents
type: specification
summary: "A fact document is the JSON form of a list of facts: each fact names its subjectset's theory, resource and relation under their own keys, and its subject as an object whose keys say which kind of subject it is."
status: evolving
cites:
  - "[[fact]]"
  - "[[subjectset]]"
  - "[[subject]]"
  - "[[identity]]"
  - "[[resource]]"
  - "[[relation]]"
  - "[[theory]]"
---

# Fact documents

A fact document is the JSON form of a list of [[fact]]s. Each fact binds a [[subject]] to a [[subjectset]]. Facts record memberships outright, and a [[theory]]'s rewrites derive further memberships from them. A fact is a value built from typed parts; a document is notation for it.

Every part of a fact has its own JSON key, so Sungma splits no string to read one. Resources and identities are opaque, and an email or a URI is a valid one.

## Sample

```json
[
  { "theory": "file", "resource": "readme", "relation": "owner",
    "subject": { "identity": "anne@example.com" } },
  { "theory": "folder", "resource": "root", "relation": "viewer",
    "subject": { "theory": "group", "resource": "eng", "relation": "member" } },
  { "theory": "file", "resource": "readme", "relation": "parent",
    "subject": { "theory": "folder", "resource": "root" } }
]
```

## Document

- A document is one JSON array of facts. An empty array holds no facts.
- A fact is an object with four keys: `theory`, `resource` and `relation` name its subjectset, and `subject` holds its subject.
- A subject is an object, and its keys say which kind it is:
  - `identity` alone is an [[identity]].
  - `theory` and `resource` are a [[resource]], called a resource member as a subject.
  - `theory`, `resource` and `relation` are a subjectset.
- A subject with any other set of keys is refused, such as `identity` with `theory`, or `theory` without `resource`.
- A fact missing a key, a key written twice, and a key the document doesn't define are refused.
- Every name is a string.
- Key order and fact order carry no meaning.

## Names

A theory name and a [[relation]] name follow the grammar in [Theory documents](theory-documents.md#names).

A resource and an identity are owned by the caller: a natural key, a surrogate key, a GUID, a path or an email address. Sungma compares them and never interprets them. Each one:

- is at least one character;
- holds no whitespace;
- is case-sensitive, so `Anne` and `anne` are two identities.

Sungma's other interfaces, such as the check API and error messages, write a resource as `theory:resource` and a subjectset as `theory:resource#relation`. That text splits at the first `:` and the last `#`, because theory and relation names allow neither. A fact document doesn't use it.

## Limits

A fact is refused if it breaks any of these limits:

- A theory name and its resource, written `theory:resource`, are at most 256 bytes.
- An identity is at most 512 bytes.
- A theory or relation name is at most 64 bytes.

## Errors

A JSON library reads the document. A document that isn't valid JSON, and a fact that breaks a rule above, are refused with an error that carries its line and column. An error that quotes a name quotes at most 64 bytes of it, with control and other invisible characters escaped, as theory document errors do.

A refused document declares no facts.
