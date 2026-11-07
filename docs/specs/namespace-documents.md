---
title: Namespace documents
type: specification
summary: "A namespace document is the JSON or TOML form of one namespace: its name, each relation's rewrite written as an expression string, and the subjects each relation's facts may bind. Both forms parse to the same namespace."
status: evolving
cites:
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[subjectset]]"
  - "[[fact]]"
  - "[[resource]]"
  - "[[subject]]"
  - "[[closure]]"
  - "[[identity]]"
  - "[[theory]]"
---

# Namespace documents

A namespace document is the JSON or TOML form of one [[namespace]]. Both forms parse to the same namespace. A document maps the namespace's name to its [[relation]]s, and each relation's name to a [[rewrite]] written as an expression string. A relation whose facts bind subjects also declares which [[subject]]s they may bind. A namespace is the unit a write declares, so a document holds exactly one. Sungma parses a document into a typed rewrite tree for each relation, checks the namespace whole, and prints a namespace back into a document, in either form, that parses as an equal namespace.

## The name

The namespaces that model one domain together form a [[theory]], in the logician's sense: a set of sentences that, together with the [[fact]]s, settles every question it can be asked. Here the sentences are rewrites, and each question asks whether a [[subject]] is in the [[closure]] of a [[subjectset]]. The rules in [Checks](#checks) exist to keep that true, so a theory and its facts give exactly one answer to each question.

## Sample

The `file` namespace: files sit in folders, and `(parent, viewer)` reads the viewers of the folder a file sits in, under the `folder` namespace. People and clients are identities, and a group's members can be given a role as `group#member`.

```json
{
  "file": {
    "owner": { "subjects": ["identity"] },
    "parent": { "subjects": ["folder"] },
    "editor": { "rewrite": "this | owner", "subjects": ["group#member", "identity"] },
    "viewer": { "rewrite": "(this | editor | (parent, viewer)) ! banned", "subjects": ["group#member", "identity"] },
    "auditor": { "rewrite": "this & viewer", "subjects": ["identity"] },
    "banned": { "subjects": ["identity"] }
  }
}
```

The same namespace in TOML:

```toml
[file]
owner = { subjects = ["identity"] }
parent = { subjects = ["folder"] }
editor = { rewrite = "this | owner", subjects = ["group#member", "identity"] }
viewer = { rewrite = "(this | editor | (parent, viewer)) ! banned", subjects = ["group#member", "identity"] }
auditor = { rewrite = "this & viewer", subjects = ["identity"] }
banned = { subjects = ["identity"] }
```

## JSON form

- A document is one JSON object with exactly one key, the namespace's name. Its value is an object mapping the namespace's relation names to their expressions.
- A relation's value is its expression, a non-empty string, or an object that also declares the subjects it accepts, as [Accepted subjects](#accepted-subjects) says. Any other value, such as `null`, `""`, a number or an array, is refused.
- A namespace without relations is written `{}`. It names a kind of [[resource]] that is only ever a subject, `namespace:id`, of facts in other namespaces, as a `user` type does in other ReBAC models. Any other value for the namespace, such as `null` or `[]`, is refused.
- A document declares exactly one namespace. An empty document or `{}` declares nothing, and an object with two or more keys declares more than one, so both are refused as parse errors. An API that takes a document answers either as a bad request.
- An object with the same key twice is refused, including a relation named twice. JSON leaves duplicate keys to the reader, and a reader that keeps the last one would hide a mistake.
- Key order carries no meaning.
- Names and expressions hold no JSON escapes, such as `\n` or `\u006f`. Nothing a name or an expression needs requires one, and without them a column in an expression is also a column in the document. A string with an escape is refused.
- A document carries no grammar version. The REST endpoint that takes it is versioned instead.

## TOML form

- A document is one TOML table. Its header is the namespace's name, and each key in it names a relation and holds its expression.
- A dotted namespace name is quoted, as in `["drive.file"]`. TOML reads an unquoted dot as a nested table.
- A relation's value is its expression, a non-empty string, or an inline table that also declares the subjects it accepts, as [Accepted subjects](#accepted-subjects) says. Any other value, such as `true`, a number, an array or a nested table like `a.b = "x"`, is refused.
- An expression may be any TOML string: basic `"…"`, literal `'…'`, or multi-line when it wraps. Strings hold no escapes, as in the JSON form, and a string with one is refused.
- A namespace without relations is a header with no keys.
- A document declares exactly one namespace. An empty document, or one with two or more tables, is refused.
- A relation named twice, or a table header written twice, is refused. TOML itself refuses both.
- Comments are allowed. They aren't kept, because Sungma stores the namespace as the printer's JSON form.
- A document is TOML, version 1.0 or later. Like the JSON form, it carries no grammar version.

## Limits

A document is refused if it breaks any of these limits:

- A document is at most 4 MiB. The other limits allow about 2.1 MB of compact JSON; the rest is room for whitespace.
- A single namespace.
- A namespace declares at most 500 relations.
- A relation accepts at most 32 kinds of subject.
- A namespace name is at most 256 bytes.
- A relation name is at most 64 bytes.
- An expression is at most 4 KiB (4,096 bytes).

The rewrite depth limit is under [Checks](#checks).

## Names

A relation name is one name. A namespace name is one or more names joined by dots, such as `file` or `drive.file`; the dots carry no meaning. A namespace name is also the namespace part of a resource name, as `file` is in `file:design.md`. The grammars in this spec are EBNF: `::=` defines, `|` alternates, `{ }` repeats zero or more times, quoted text is literal, and `'x'…'y'` is an inclusive character range.

```ebnf
⟨namespace name⟩   ::= ⟨name⟩ { '.' ⟨name⟩ }
⟨relation name⟩ ::= ⟨name⟩     excluding 'this'
⟨name⟩          ::= ⟨name-start⟩ { ⟨name-char⟩ }
⟨name-start⟩    ::= ⟨letter⟩ | '_'
⟨name-char⟩     ::= ⟨letter⟩ | ⟨digit⟩ | '_'
⟨letter⟩        ::= 'a'…'z' | 'A'…'Z'
⟨digit⟩         ::= '0'…'9'
```

Names are case-sensitive: `Owner` and `owner` are two different names. Nothing is lowercased.

Namespace and relation names are value types, `NamespaceName` and `RelationName`, not strings. Each has two constructors:

- A checked constructor, `FromStr` and `TryFrom<&str>`, used for any name that arrives from an edge, such as a document, a request or a fact. It refuses a name that breaks the grammar or the length limit, and `RelationName` also refuses `this` in any casing.
- An unchecked constructor, `new_unchecked`, used only for names from a trusted source, such as storage, which holds only names that were checked on the way in.

Everything past the edge, including the rewrite tree, the namespace and the printer, holds these types, so a name the grammar doesn't allow can't reach them.

The keyword is `this`, in lowercase only. No relation may be named `this` in any casing, so `This` and `THIS` are refused as names, and so is a reference to one: `"viewer": "this"` is valid, and `"viewer": "This"` is refused as a reserved name. A name that differs from the keyword only in case would read as the keyword. The core accepts these names; the document reserves them.

## Rewrite grammar

```ebnf
⟨rewrite⟩             ::= ⟨union⟩
⟨union⟩               ::= ⟨intersection⟩ { '|' ⟨intersection⟩ }
⟨intersection⟩        ::= ⟨exclusion⟩ { '&' ⟨exclusion⟩ }
⟨exclusion⟩           ::= ⟨term⟩ { '!' ⟨term⟩ }
⟨term⟩                ::= 'this'
                        | ⟨computed-subjectset⟩
                        | ⟨fact-to-subjectset⟩
                        | '(' ⟨rewrite⟩ ')'

⟨computed-subjectset⟩ ::= ⟨relation name⟩
⟨fact-to-subjectset⟩  ::= '(' ⟨factset⟩ ',' ⟨computed-subjectset⟩ ')'
⟨factset⟩             ::= ⟨relation name⟩
```

The grammar defines three operators. In binding order, they are:

- `!` exclusion, which binds tightest
- `&` intersection
- `|` union

So `a | b & c` is `a | (b & c)`. Each level reads left to right, so `a ! b ! c` is `(a ! b) ! c`.

The following rules apply on top of the grammar:

- Whitespace, including line breaks, ends a token and is otherwise ignored.
- A name is the longest run of name characters, so `thisone` is a name and not `this` followed by `one`.
- A parenthesized pair with a comma is a fact-to-subjectset. Parentheses around anything else group.
- A run of one operator parses as a single node with one operand per term, so `a | b | c` is one union of three.
- Parentheses are kept as structure. `(a | b) | c` is a union whose first operand is a union, not a union of three.

## Accepted subjects

A relation whose rewrite reads `this` declares which [[subject]]s its facts may bind. A fact whose subject isn't one of them is refused, so a mistake in the data can't quietly empty a role.

Each entry in `subjects` is one of:

- `identity`: an identity, such as a person's or a client's `sub`.
- A namespace name, such as `folder`: a resource member of that namespace, such as `folder:root#...`.
- A namespace name and a relation joined by `#`, such as `group#member`: a subjectset of that relation.

```ebnf
⟨accepted subject⟩ ::= 'identity' | ⟨namespace name⟩ [ '#' ⟨relation name⟩ ]
```

`subjects` holds at least one entry and no entry twice. Order carries no meaning.

No entry accepts every subject. A relation that would take anything can't have its links checked, so each one names what it takes.

A relation that declares subjects is written as an object in JSON and an inline table in TOML, with a `subjects` key and an optional `rewrite` key, and no other keys. `rewrite` is a non-empty expression string. Left out, it is `this`:

```toml
[folder]
owner = { subjects = ["identity"] }
parent = { subjects = ["folder"] }
viewer = { rewrite = "this | owner | (parent, viewer)", subjects = ["identity", "group#member"] }
```

```json
"parent": { "subjects": ["folder"] }
```

A relation whose rewrite doesn't read `this` is written as a plain string. It accepts no facts.

Writing a namespace checks the declarations:

- A rewrite that reads `this` without declaring subjects is refused, and so is a relation that declares subjects without reading `this`.
- For each fact-to-subjectset, every namespace its first relation accepts must declare its second relation. `(parent, viewer)` is refused unless `folder` declares `viewer`.

`identity` is reserved: no namespace may be named `identity`.

## Meaning

A relation's rewrite defines the closure of each [[subjectset]] formed from that relation. Each term contributes a set of subjects, taken for the subjectset in hand:

- `this` is the subjects the [[fact]]s bind to the subjectset.
- A computed subjectset is the closure of the subjectset formed from the same resource and the named relation.
- A fact-to-subjectset reads the subjects the facts bind to the subjectset formed from the same resource and the factset. For each subject that is a resource member or a subjectset, it takes the closure of the subjectset formed from that subject's resource and the computed relation. An [[identity]] has no resource and is skipped.

A subjectset whose relation its namespace doesn't declare has an empty closure. A fact-to-subjectset reaches one when the namespace of a resource it follows lacks the computed relation.

## Checks

A namespace is checked whole when it is declared, and is refused if any of the following holds:

- A relation is declared twice.
- A rewrite nests more than 100 levels deep. A leaf is one level, a run of `|` or `&` is one level however many operands it has, and each `!` adds a level. Grouping parentheses nested more than 100 deep are also refused, before the tree is built.
- A computed subjectset, or the factset of a fact-to-subjectset, names a relation the namespace doesn't declare. The computed relation of a fact-to-subjectset belongs to the namespace of each fact's subject, so it's checked against the subjects the factset accepts, as [Accepted subjects](#accepted-subjects) says.
- A relation's accepted subjects break a rule in [Accepted subjects](#accepted-subjects).
- Computed subjectsets form a cycle, such as `"viewer": "editor"` with `"editor": "viewer"`, or `"viewer": "this ! viewer"`. A cycle through a fact-to-subjectset, such as folders inside folders, reads a fact at each step and is allowed.

## Errors

A JSON or TOML library reads the document, and a document that isn't valid in its form is refused with the library's error and its line and column. Nothing further is checked.

Sungma's own parser reads each expression. It reports the problems it finds in document order, up to 50; past that, a final error says there were too many. Each error carries the relation it was found in and, for a problem inside an expression, the column where the problem starts. A column counts characters from 1, from the start of the expression. In a TOML multi-line string, it counts across the string's line breaks, so it doesn't match a column on the document's lines. A problem found only by checking a whole namespace, such as a cycle, is reported at the relation where it is found.

One mistake reports one error:

- An expression stops at its first syntax error, and parsing goes on with the next relation.
- A relation whose expression fails to parse is still declared, so a reference to it isn't dangling. The cycle check skips it.
- Relations that reach each other through computed subjectsets, a strongly connected component, report one cycle error between them, however many cycles they form. It is reported at whichever of them the check meets first, and carries a path from that relation back to itself.

The parser refuses a document if it finds any problem, and declares nothing from it.

Each error is a kind, with its relation and column where it has them. The kind is a variant of one enum with a variant for each problem this spec names, such as a relation whose value isn't a string or the accepted-subjects form, a reserved name, an accepted subject that isn't allowed, a broken limit, a rewrite syntax error, a dangling reference or a cycle. A variant carries the names involved. Callers match on the variant. There are no string codes.

An error that quotes the document quotes at most 64 bytes of it, cut at a character boundary and marked as cut, with control characters and other invisible or reordering characters, such as U+202E (right-to-left override) and U+200B (zero-width space), written as escapes. Errors reach logs, terminals and UIs. A quote copied as it was sent could forge a log line, recolor a terminal, reorder the text around it, or make an error as large as the document.

## Printing

The printer writes a document, in either form, that the parser reads back as an equal namespace. Sungma stores the JSON form, and Author, Sungma's UI, shows the TOML form.

- A relation that accepts no facts prints as its expression, `"name": "expression"` in JSON and `name = "expression"` in TOML.
- A relation that accepts subjects prints in the accepted-subjects form, `"name": { "rewrite": "expression", "subjects": [...] }` in JSON and `name = { rewrite = "expression", subjects = [...] }` in TOML. `rewrite` is left out when it is `this`, and subjects print sorted, comparing bytes.
- A namespace without relations prints as `"name": {}` in JSON and as its header alone in TOML. A dotted namespace name prints quoted in TOML.
- Operators print with a space on each side, and a fact-to-subjectset prints as `(factset, computed)`.
- The printer adds parentheses only where the grammar needs them:
  - A union inside a union is parenthesized.
  - A union or intersection inside an intersection is parenthesized.
  - A union or intersection on the left of `!` is parenthesized.
  - Any operator node on the right of `!` is parenthesized.
- Relations print sorted by name, comparing bytes. Order carries no meaning, so a namespace holds its relations unordered, and the printer sorts them so one namespace always prints as the same text.
- JSON objects indent by two spaces, one key per line. The TOML form prints one key per line under its header. Lines end in `\n` on every platform.
- Printing refuses, with a `PrintError`, a name made with `new_unchecked` that breaks the grammar, including `this`, or a union or intersection with one operand.

## Open

- When a fact-to-subjectset's first relation accepts a namespace that doesn't exist yet, whether the namespace write is refused or the check waits until that namespace is written. Two namespaces that reach each other can't both be written first.
- How a request names its document's form. A write carries the document inside a request body, so the form can't come from the request's content type. The endpoints that take documents settle it.

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents. Each class below is required.

1. **Document shape.** Each rule in [JSON form](#json-form) and [TOML form](#toml-form) holds, for valid documents and for each way of breaking it.
2. **Names.** The name grammar, dotted namespace names, and the reserved `this` in every casing.
3. **Grammar.** Precedence, left-to-right reading, runs as one node, parentheses kept as structure, whitespace, and longest-match names.
4. **Checks.** Each check in [Checks](#checks) refuses what it should and allows what it should, including cycles through a fact-to-subjectset and relations that share a target.
5. **Errors.** Every problem is reported in document order, up to the cap, each with its place and a message that names what was expected and what was found. Quoted input is bounded and escaped.
6. **Limits.** Each limit at its value and one past it.
7. **Printing.** The canonical form, only the parentheses the grammar needs, and each refusal: a union or intersection with one operand, and a name made with `new_unchecked` that breaks the grammar.
8. **Round trips.** Property tests from trees and from expression text. Print then parse gives an equal namespace, and printing again gives the same text. Generated trees reach the depth limits and include wide runs.
9. **Hostile input.** Within the size limit: the longest chain of computed subjectsets, the largest set of relations that reach each other, and far more relations than the limit. Each is refused or accepted without a crash and within a time bound. A namespace built in code, which bypasses the document limits, passes the same checks without a crash.
10. **Fuzzing.** A `cargo-fuzz` target, and a property test in CI, assert the parser never panics, overflows its stack or aborts on arbitrary input.
11. **Text and encoding.** Columns after multibyte characters, NUL, a UTF-8 byte-order mark (refused), and deeply nested JSON values (refused as an error, not a crash).
12. **Mutation testing.** `cargo mutants` leaves no surviving mutant in the parser, the printer or the namespace checks.

### Cases

The cases each class covers, at minimum.

#### Document shape

1. The sample parses to the namespace it shows.
2. A document declares exactly one namespace: `{}`, an object with two namespace keys, and a namespace key written twice are refused.
3. A namespace without relations, `{}`, parses. A namespace whose value isn't an object is refused.
4. A relation's value is a non-empty string, or the accepted-subjects form. `null`, `""`, numbers, arrays, and objects of any other shape are refused.
5. A relation named twice is one error.
6. Text that isn't valid JSON or TOML is refused with its library's error.
7. JSON escapes are handled as the [JSON form](#json-form) section says.
8. The TOML sample parses to the same namespace as the JSON sample.
9. A TOML document with no table, two tables, a non-string value or a nested table is refused, and so is a string with an escape. Basic, literal and multi-line strings each parse.
10. A dotted namespace name parses quoted. Unquoted, it's refused as a nested table.
11. Comments are allowed anywhere TOML allows them, and change nothing.
12. The accepted-subjects form parses in each form, with and without `rewrite`. A key other than `rewrite` or `subjects`, an empty `subjects`, an entry written twice, an entry that breaks the grammar, and an empty `rewrite` are each refused.

#### Names

1. Namespace and relation names follow the grammar, and names that break it are refused.
2. A dotted namespace name is one name. `.a`, `a.`, `a..b` and `a.1b` are refused.
3. `this` in any casing is refused as a relation name, and in any casing other than `this` it's refused inside an expression.
4. `NamespaceName` and `RelationName`'s `FromStr` and `TryFrom<&str>` refuse every name the grammar or the length limit refuses, including names holding `"`, `:`, `#`, whitespace or control characters.

#### Grammar

1. `!` binds tighter than `&`, and `&` tighter than `|`.
2. `!` reads left to right.
3. A run of one operator is one node.
4. Parentheses are kept as structure.
5. Whitespace only separates tokens.
6. A name is the longest run of name characters.
7. A syntax error carries the column where it starts, and its message names what was expected and what was found.

#### Checks

1. A relation declared twice, a computed subjectset or factset the namespace doesn't declare, and a cycle of computed subjectsets are refused, including a relation that excludes itself.
2. A cycle through a fact-to-subjectset is allowed, and so are relations that share a target.
3. The computed relation of a fact-to-subjectset is refused when a namespace its factset accepts doesn't declare it.
4. A dangling target is reported once for each relation that names it.
5. Relations that reach each other are one cycle, reported at the relation where the check meets the cycle first, with a path back to that relation.
6. A rewrite tree may be 100 levels deep and no deeper, whether the extra level is under a run, an exclusion's base or its excluded side.
7. Redeclaring a relation checks the whole namespace again.
8. A rewrite that reads `this` without `subjects`, and `subjects` on a rewrite without `this`, are refused. `rewrite` left out means `this`.
9. A fact whose subject isn't among its relation's accepted subjects is refused, and so is a fact under a relation that accepts none.
10. A namespace named `identity` is refused.

#### Errors

1. Every problem is reported, in document order.
2. Errors stop at the cap, and the last error says more were found.
3. Each error's message names its relation and, inside an expression, its column.
4. An error quoting a name or text longer than 64 bytes, or holding control, zero-width or right-to-left characters, quotes at most 64 bytes with those characters escaped. This includes the error that refuses a JSON escape.

#### Limits

1. Each limit is tested at its value and one past it: a 256-byte namespace name and a 257-byte one, a 64-byte relation name and a 65-byte one, a 4,096-byte expression and a 4,097-byte one, 500 relations and 501, 32 accepted subjects and 33, a document of exactly the maximum size and one byte more, 100 levels of nesting and 101, and 100 grouping parentheses and 101.
2. A dotted namespace name of exactly 256 bytes is accepted.

#### Printing

1. The printer writes the canonical form of the sample, and of a namespace without relations, in each form. `rewrite` is left out when it is `this`, and subjects print sorted.
2. The printer adds only the parentheses the grammar needs.
3. The printer refuses, as a caller defect, a union or intersection with one operand, and a name made with `new_unchecked` that breaks the grammar or is `this`, so it never writes a document that parses differently or isn't valid in its form.

#### Round trips

1. A property test builds namespaces, prints them in each form, parses them back to equal namespaces, and prints the same text again. A namespace printed in one form and parsed from the other is equal too. The generated trees include both depth limits and wide runs of one operator, and the generated relations include accepted subjects of every kind.
2. A property test generates expression strings from the grammar's tokens and checks that parse, print and parse again gives an equal tree.

#### Hostile input

1. A chain of computed subjectsets as long as the document limit allows, such as `"r0": "r1"`, `"r1": "r2"` and so on, is refused without a crash.
2. One large set of relations that all reach each other is refused without a crash.
3. Many more relations than the relation limit allows are refused without a crash.
4. Each of these finishes within a time bound that catches quadratic work.
5. A namespace built in code, which bypasses the document limits, passes the same chain and cycle checks without a crash.

#### Fuzzing

1. A `cargo-fuzz` target runs the parser on arbitrary bytes.
2. A property test runs the parser on arbitrary strings in CI.
3. Neither panics, overflows its stack or aborts.

#### Text and encoding

1. Columns count characters correctly after multibyte characters such as `é` and emoji.
2. A NUL character is refused: a raw NUL in a string by the JSON library, and an escaped `\u0000` as an escape.
3. A UTF-8 byte-order mark before the document is refused.
4. A relation value that is a deeply nested array or object is refused as an error, not a crash.

#### Mutation testing

1. `cargo mutants` leaves no surviving mutant in the parser, the printer or the namespace checks.
