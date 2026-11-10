---
title: Theory documents
type: specification
summary: "A theory document declares one or more namespaces in TDL, Theory Definition Language. An optional theory name prefixes unqualified namespace names. Each namespace is checked and stored on its own, under its full name."
status: evolving
cites:
  - "[[theory]]"
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[subjectset]]"
  - "[[fact]]"
  - "[[resource]]"
  - "[[subject]]"
  - "[[closure]]"
  - "[[identity]]"
  - "[[sungma-authorization]]"
---

<!-- final -->
# Theory documents

A theory document declares one or more namespaces in TDL, Theory Definition Language. Any set of namespaces is a theory. Each relation declares its constraints, the kinds of subject its facts may bind, and its rewrite. Files end in `.theory`.

A document is applied, like a DDL script: Sungma checks each namespace and stores it under its fully qualified name.

## Why a theory

In logic, a theory is a set of sentences closed under deduction. Here the sentences are rewrites. When rewrites are combined with fact tuples, a graph is formed from which each subjectset's closure may be deduced. [Checks](#checks) keep that true.

## TDL

> [!NOTE]
> The grammars are ISO EBNF: `=` defines, `;` ends a rule, `,` joins in sequence, `|` separates alternatives, `( )` groups, `[ ]` is optional, `{ }` repeats, `-` excludes, and `? ?` describes in words.

A relation reads like a generic type: its name, its constraints in angle brackets, then a colon and its rewrite. `this` is implicit when no rewrite is provided. With no constraints, it's computed. A relation needs constraints, a rewrite, or both.

```ebnf
document       = namespace, { namespace }, { theory section }
               | theory section, { theory section } ;
theory section = 'theory', ':', theory name, namespace, { namespace } ;
namespace      = 'namespace', ':', namespace name, { relation } ;
relation       = relation name, ( '<', constraints, '>', [ ':', rewrite ] | ':', rewrite ) ;
constraints    = constraint, { ',', constraint } ;
```

- Whitespace, line breaks included, only separates tokens. A rewrite may start on the next line and wrap.
- Whitespace is a space, a tab, a CR or an LF, and a line ends in LF or CRLF. Any other whitespace, such as a form feed or a non-breaking space, is refused.
- Names and constraints have no spaces inside them, so `a # b`, `a .b`, `.. b` and `@ identity` are refused.
- The headers `namespace:` and `theory:` are written in lowercase.
- A rewrite runs until the next relation (a name followed by `<` or `:`), the next `namespace:` or `theory:`, or the end.
- `//` starts a comment to the end of the line. Comments aren't kept.
- There are no quotes, escapes, numbers or booleans, so every token is a name, a keyword or punctuation.
- A document is UTF-8 without a byte-order mark. Outside comments, a character the grammar doesn't use is refused.
- A document declares at least one namespace.

## Names

A name starts with a letter or an underscore, followed by letters, digits and underscores. Names are case-sensitive.

```ebnf
letter = ? an ASCII letter, 'a' to 'z' or 'A' to 'Z' ? ;
digit  = '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' ;
name   = ( letter | '_' ), { letter | digit | '_' } ;
```

A namespace name is one or more names joined by dots, such as `file` or `drive.file`. A first segment of `sungma` is reserved for Sungma's own namespaces, so `sungma.audit` is reserved and `sungmax` isn't.

```ebnf
namespace name = name, { '.', name } ;
```

A theory name is one name: the first segment of its namespaces' names. A `theory:` line opens a section of the document, as [The theory name](#the-theory-name) says.

```ebnf
theory name = name ;
```

A relation name is one name. `this` is the rewrite keyword, and `namespace` and `theory` are TDL keywords, so no relation may be named any of them, in any casing: `This | owner` is refused. `namespace` and `theory` may still name namespaces and appear in constraints.

```ebnf
relation name = name - ? 'this', 'namespace' or 'theory', in any casing ? ;
```

Names are parsed at the edge.

## Constraints

A relation's constraints limit which subjects can be bound to its subjectsets through a fact.

A relation whose rewrite reads `this` takes facts, and declares at least one constraint. A computed relation's rewrite doesn't read `this`, so it takes no facts and declares no constraints.

There are three kinds of constraint, one for each kind of subject:

- `@identity`: an identity, such as a person's or a client's `sub`. A name can't start with `@`, so `@identity` can never be a namespace;
- a namespace, such as `folder`: a resource member of it, such as `folder:root#...`;
- `namespace#relation`, such as `group#member`: a subjectset of that relation.

```ebnf
constraint = '@identity' | [ '..' ], namespace name, [ '#', relation name ] ;
```

Constraints can be listed in any order, but no constraint may appear twice. Two constraints are the same when their fully qualified names are, so under `theory: drive`, `<file, ..drive.file>` lists one twice.

## Rewrite grammar

```ebnf
rewrite             = union ;
union               = intersection, { '|', intersection } ;
intersection        = exclusion, { '&', exclusion } ;
exclusion           = term, { '!', term } ;
term                = 'this' | computed subjectset | fact to subjectset | '(', rewrite, ')' ;
computed subjectset = relation name ;
fact to subjectset  = '(', factset, ',', computed subjectset, ')' ;
factset             = relation name ;
```

`!` (exclusion) binds tightest, then `&` (intersection), then `|` (union), each reading left to right: `a | b & c` is `a | (b & c)`, and `a ! b ! c` is `(a ! b) ! c`.

- A name is the longest run of name characters, so `thisone` is one name.
- A run of one operator is one node: `a | b | c` is one union of three.
- Parentheses are kept: `(a | b) | c` is a union whose first operand is a union.

## Meaning

A relation's rewrite defines the closure of each of its subjectsets. Each kind of term adds subjects:

- `this` adds the subjects bound by the subjectset's own facts.
- A computed subjectset adds the closure of the named relation on the same resource.
- A fact-to-subjectset reads the facts on the same resource under the factset relation. For each subject they bind, it adds the closure of the followed relation on that subject's resource. An identity has no resource, so it adds nothing.

Each term produces a set of subjects, and operators combine those sets: 

- `|` : union
- `&` : intersection
- `!` : difference.

Evaluating a rewrite applies the set of facts to the relation's definition, and the result is the closure. A subjectset whose relation isn't declared has an empty closure.

## The theory name

The theory name is a namespace prefix. The parser prepends it and a dot to each namespace name in headers and constraints, to form a fully qualified name. Under `theory: sungma`, `namespace: root` means `namespace: sungma.root`, and `group#member` means `sungma.group#member`.

- A `theory:` line applies to every namespace name below it until the next `theory:` line or the end of the document, so one document can hold several theories. Under `theory: x`, `y.z` means `x.y.z`. A theory may open more than one section.
- Namespaces before the first `theory:` line get no prefix. Their names are written fully qualified.
- Dotted and undotted always describe the fully qualified name: `a` is undotted, and `c.d` is dotted, even when it's written `d` under `theory: c`.
- In a constraint, `..` before a namespace name means the name is a fully qualified namespace outside the theory. Under `theory: drive`, `<file>` means `drive.file`, `<..file>` means `file`, and `<..io.file#owner>` means `io.file#owner`. Outside any theory, `..` changes nothing.
- Rewrites name only relations, so the prefix never touches them.

Once parsed, every namespace name is fully qualified.

## Sample

[Sungma's own authorization](sungma-authorization.md#the-model):

```tdl
theory: sungma

// The deployment, one resource: sungma.root:sungma
namespace: root
  owner<@identity>
  admin<@identity>: this | owner
  namespace_creator<@identity, group#member>: this | admin
  group_creator<@identity, group#member>: this | admin
  operator<@identity, group#member>: this | admin
  auditor<@identity, group#member>: this | admin
  reader<@identity, group#member>: this | auditor
  writer<@identity, group#member>: this | admin
  fact_writer<@identity, group#member>: this | admin
  fact_reader<@identity, group#member>: this | fact_writer | auditor
  directory_reader<@identity, group#member>: this | auditor
  access_reader: auditor

namespace: namespace
  parent<root>
  protected<root>
  owner<@identity, group#member>: this | (parent, owner) ! (protected, owner)
  admin<@identity, group#member>: this | owner | (parent, admin) ! (protected, admin)
  writer<@identity, group#member>: this | admin | (parent, writer) ! (protected, writer)
  reader<@identity, group#member>: this | writer | fact_reader | auditor | (parent, reader)
  fact_writer<@identity, group#member>: this | admin | (parent, fact_writer) ! (protected, fact_writer)
  fact_reader<@identity, group#member>: this | fact_writer | auditor | (parent, fact_reader)
  auditor<@identity, group#member>: this | (parent, auditor)
  access_reader: admin | auditor

namespace: relation
  parent<namespace>
  writer<@identity, group#member>: this | (parent, fact_writer)
  reader<@identity, group#member>: this | writer | (parent, fact_reader)

namespace: group
  parent<root>
  owner<@identity, group#member>: this | (parent, admin)
  member<@identity, group#member>
  access_reader: owner | (parent, auditor)
```

## Checks

A document is rejected if it breaks a rule of TDL, declares one namespace twice by its full name, or holds a namespace that fails a check:

- a relation is declared twice;
- a rewrite nests deeper than the depth limit (a leaf or a run of `|` or `&` is one level, and each `!` adds one), or groups parentheses deeper than it;
- a computed subjectset or a factset names an undeclared relation;
- a rewrite reads `this` without constraints, or a relation declares constraints without reading `this`;
- a constraint names its own namespace with a relation it doesn't declare;
- a fact-to-subjectset's factset names its own namespace in a constraint, and the namespace doesn't declare the followed relation: in `folder`, with `parent<folder>`, `(parent, viewer)` needs `folder` to declare `viewer`;
- computed subjectsets form a cycle, such as `viewer: editor` with `editor: viewer`. A cycle through a fact-to-subjectset, such as folders in folders, is allowed.

The parser checks each namespace on its own. It doesn't check references to other namespaces, in the document or outside it.

## Limits

- Rewrite depth: a parser setting, 100 by default.
- Errors: a parser setting, 50 by default.
- A full namespace name: 256 bytes.
- A relation name: 64 bytes.

## Errors

Errors are reported as they are encountered in the document, up to the error limit, then one that says there were more. Each carries a line, a column counted in characters from 1, and the namespace's full name, where it has them. A whole-namespace problem, such as a cycle, is reported at its relation:

- A syntax error stops its rewrite, and parsing goes on with the next relation.
- A relation whose rewrite fails is still declared, so references to it don't dangle.
- Relations that reach each other report one cycle, at the first one the check meets, with the full path back to it.

Each error is a variant of one enum, carrying the names involved. There are no string codes.

An error quotes at most 64 bytes, cut at a character boundary and marked as cut, with control and invisible or reordering characters, such as U+202E and U+200B, escaped. Errors reach logs, terminals and UIs, where a raw quote could forge a line, recolor a terminal or reorder text. Names in errors are quoted too, so a long namespace name can appear cut.

## Printing

The printer only prints. It doesn't check what it's given, and it never refuses. Namespaces the parser accepted print as a document that parses back as equal namespaces, and the same namespaces always print as the same text.

- Undotted namespaces print first, sorted, with no `theory:` line above them. They must come first, or they would read as part of the theory above them.
- Then each theory prints once, sorted by name: its `theory:` line, then its namespaces, sorted, with the theory name and its dot taken off. `x.y.z` prints as `y.z` under `theory: x`. A constraint that names a namespace outside its own namespace's theory prints with `..`.
- Within a namespace, relations print sorted by name, and constraints sorted as written, each comparing bytes. An author's order, comments and line breaks are lost.
- Each relation is one line, indented two spaces, with one space after each colon and comma and around each operator. `: this` is left out, since `this` is implicit when no rewrite is provided.
- Parentheses print only where the grammar needs them: a union inside a union, a union or intersection inside an intersection or on the left of `!`, and any operator on the right of `!`.
- A blank line follows each `theory:` line, and one comes before each `theory:` line after the start of the document. Namespaces follow one another with no blank line between them.
- Lines end in `\n`.
- The printer has no size limit, and prints a rewrite of any depth.

Author, Sungma's UI, shows and edits namespaces in TDL.
<!-- /final -->

## Open

1. **Writing a document.** One write per namespace, in turn, or one atomic write, which would come with [write ids](../notes/writes-commit-through-a-write-id.md).
2. **Namespaces that reference each other.** Neither can be written first while a reference to a missing namespace is refused.
3. **The `sungma` theory.** Whether only Sungma releases write in it.

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents.

#### Documents

1. The sample parses to Sungma's four namespaces under their full names.
2. An empty document, or one of only comments, is refused. A namespace without relations parses.
3. `this` is implicit when no rewrite is provided, and a relation with no constraints is computed.
4. A rewrite on its relation's line, on the next line, or wrapped parses the same.
5. A comment at the end of any line changes nothing. A `#` outside a constraint is refused.
6. A quote, an escape, a number, a NUL, a byte-order mark, or any other character outside the grammar is refused.
7. A rewrite left open, or a missing namespace name, doesn't swallow the next relation or `namespace:`.
8. A document with one bad namespace declares none.

#### Theory name

1. The prefix applies to every namespace name in a section's headers and constraints, never to `@identity` or rewrites.
2. Under a theory, `..` before a constraint's namespace name means a fully qualified namespace outside the theory, and the printer writes `..` where a constraint points outside its namespace's theory.
3. Each `theory:` line applies until the next one, and namespaces before the first get no prefix.
4. A theory name with a dot is refused.

#### Names

1. Names follow the grammar. `.a`, `a.`, `a..b` and `a.1b` are refused.
2. `this`, `namespace` and `theory` in any casing are refused as relation names; `namespace` and `theory` are allowed as namespace names and in constraints.
3. The `sungma` first segment is enforced at write time, as [Sungma's own authorization](sungma-authorization.md#protected-namespaces) says, so the parser reads Sungma's own namespaces.
4. Checked names refuse `"`, `:`, `#`, whitespace and control characters.

#### Grammar

1. Precedence, left-to-right reading, runs as one node, kept parentheses, and longest-match names.
2. A syntax error names what was expected and what was found.

#### Checks

1. Each check in [Checks](#checks) refuses what it should, including a relation that excludes itself, and allows cycles through a fact-to-subjectset and relations that share a target.
2. An empty constraint list, an entry twice, or a malformed entry is refused.
3. A dangling target within a namespace is reported once per relation that names it, and a set of relations that reach each other is one cycle error with the full path.
4. A constraint or fact-to-subjectset that names another namespace isn't checked, whether that namespace is in the document or not.

#### Errors

1. Errors come in document order and stop at the cap.
2. Lines and columns are right after multibyte characters and wrapped rewrites, and each error names its namespace.
3. Quotes are bounded and escaped.
4. A bad constraint reports one error, without reading a later constraint as a relation.

#### Limits

1. Each limit at its value and one past it, including the depth limit for levels and for parentheses, a setting other than the default, and a 256-byte dotted name.

#### Printing

1. The canonical form, `: this` left out, sorted constraints, and only needed parentheses.
2. Undotted namespaces print first with no `theory:` line, then each theory once, sorted, with its namespaces under it and the prefix taken off.
3. The printer prints whatever it's given without refusing: an empty set, a namespace given twice, a rewrite far past the depth limit, and a document past 2 MiB.

#### Round trips

1. Generated namespaces, with no theory, one, and several, reaching both depth limits, with wide runs and every kind of constraint: print, parse and print again gives equal namespaces and the same text.
2. Rewrite text generated from the grammar's tokens parses, prints and parses to an equal tree.

#### Hostile input

1. A chain of 100,000 computed subjectsets is accepted, and the largest set of mutually reaching relations is refused. Each finishes without a crash, within a bound that catches quadratic work.
2. A namespace built in code, far larger than any written document, passes the same checks without a crash.
3. A fuzz target and a CI property test run the parser on arbitrary input without a panic, stack overflow or abort.
4. Mutation testing leaves no surviving mutant in the parser, the printer or the checks.
