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

## Writes

A fact is never changed. A write adds facts and retires facts, and a retired fact stays in the history at the revisions it held. A write request holds two fact documents:

```json
{
  "add": [
    { "theory": "file", "resource": "readme", "relation": "parent",
      "subject": { "theory": "folder", "resource": "b" } }
  ],
  "retire": [
    { "theory": "file", "resource": "readme", "relation": "parent",
      "subject": { "theory": "folder", "resource": "a" } }
  ]
}
```

- Both keys are optional, and a request with neither is refused.
- The write applies both lists at one revision, so no read sees one half. Moving a file from one folder to another is one write.
- Adding a fact already held, or retiring a fact not held, changes nothing.
- A request that breaks a rule in either list is refused whole and writes nothing.

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

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents. Each class below is required.

1. **Document shape.** Each rule in [Document](#document) holds, for valid documents and for each way of breaking it.
2. **Subjects.** Each kind of subject is read from its keys, and every other set of keys is refused.
3. **Names.** Theory and relation names follow the theory document grammar, including the reserved `this`.
4. **Resources and identities.** Opaque text is accepted, whitespace and empty text are refused, and case is kept.
5. **Limits.** Each limit at its value and one past it.
6. **Errors.** Each error carries the line and column of the value at fault, and quoted text is bounded and escaped.
7. **All or nothing.** A refused document declares no facts and interns no names.
8. **Round trips.** Generated documents load, and each fact is found under the names it was written with.
9. **Hostile input.** Deeply nested values and large documents are refused or accepted without a crash and within a time bound.
10. **Text and encoding.** JSON escapes, multibyte characters, NUL and a byte-order mark.
11. **Fuzzing.** A `cargo-fuzz` target, and a property test in CI, assert the parser never panics, overflows its stack or aborts on arbitrary input, and that a refused document declares no facts.
12. **Mutation testing.** `cargo mutants` leaves no surviving mutant in the parser or the name checks.

[Writes](#writes) are tested with the endpoint that takes them.

### Cases

The cases each class covers, at minimum.

#### Document shape

1. The sample loads.
2. An empty array loads and holds no facts.
3. A document that isn't an array, and a fact that isn't an object, are refused.
4. A fact missing `theory`, `resource`, `relation` or `subject` is refused, naming the missing key.
5. A key written twice is refused as a duplicate, even when its second value is also invalid.
6. A key the document doesn't define is refused, in a fact and in a subject.
7. A name that isn't a string is refused.

#### Subjects

1. `identity` alone, `theory` and `resource`, and `theory`, `resource` and `relation` load as an identity, a resource member and a subjectset.
2. An empty subject, `identity` with any other key, `theory` without `resource`, `resource` without `theory`, and `relation` without both are refused.
3. A subject that isn't an object is refused.

#### Names

1. A theory or relation name that breaks the grammar is refused.
2. `this`, in any casing, is refused as a relation name in a fact and in a subjectset subject.

#### Resources and identities

1. Resources and identities holding `:`, `#`, `@`, `/`, `\`, `+`, `=`, `>` and non-ASCII characters load, and are found under the same text.
2. Whitespace anywhere in a resource or an identity is refused, including a space, a tab, a line break, a no-break space and an em space.
3. An empty resource and an empty identity are refused.
4. `Anne` and `anne` are two identities.

#### Limits

1. `theory:resource` of 256 bytes loads and of 257 is refused.
2. An identity of 512 bytes loads and of 513 is refused.
3. A theory or relation name of 64 bytes loads and of 65 is refused.

#### Errors

1. An error in a value carries the line and column of that value, on a document spread over several lines.
2. An error quoting a resource or an identity longer than 64 bytes, or holding control, zero-width or right-to-left characters, quotes at most 64 bytes with those characters escaped.

#### All or nothing

1. A document whose last fact is refused declares none of its facts.
2. No name from a refused document is interned.

#### Round trips

1. A property test generates documents from valid names and opaque text, loads them, and finds each fact under its names.

#### Hostile input

1. A value nested far deeper than JSON readers allow, in a fact or in a subject, is refused as an error, not a crash.
2. A document of 100,000 facts loads within a time bound.

#### Text and encoding

1. A JSON escape is decoded before the rules apply, so ` ` in a resource is whitespace and is refused, and `\` is a backslash.
2. A raw NUL is refused by the JSON library, and an escaped `\u0000` is accepted as an opaque character.
3. A UTF-8 byte-order mark before the document is refused.

#### Fuzzing

1. A `cargo-fuzz` target runs the parser on arbitrary bytes, and asserts that a refused document declares no facts.
2. A property test runs the parser on arbitrary strings in CI.
3. Neither panics, overflows its stack or aborts.

#### Mutation testing

1. `cargo mutants` leaves no surviving mutant in `sungma-lang`'s `fact.rs` or `sungma`'s `name.rs`.
