# Sample

Three namespaces for documents: files sit in folders, folders sit in folders, and groups hold members. Each is a namespace document:

```json
{
  "file": {
    "auditor": "this & viewer",
    "banned": "this",
    "editor": "this | owner",
    "owner": "this",
    "parent": "this",
    "viewer": "(this | editor | (parent, viewer)) ! banned"
  }
}

{
  "folder": {
    "banned": "this",
    "owner": "this",
    "parent": "this",
    "viewer": "(this | (parent, viewer)) ! banned"
  }
}

{
  "group": {
    "member": "this"
  }
}
```

Some facts under it, as a fact document (`crates/sungma/tests/fixtures/docs.facts.json` holds them all):

```json
[
  { "namespace": "group", "resource": "eng", "relation": "member", "subject": { "identity": "alice" } },
  { "namespace": "group", "resource": "eng", "relation": "member", "subject": { "identity": "bob" } },
  { "namespace": "folder", "resource": "root", "relation": "viewer",
    "subject": { "namespace": "group", "resource": "eng", "relation": "member" } },
  { "namespace": "folder", "resource": "root", "relation": "viewer", "subject": { "identity": "erin" } },
  { "namespace": "folder", "resource": "specs", "relation": "parent",
    "subject": { "namespace": "folder", "resource": "root" } },
  { "namespace": "folder", "resource": "specs", "relation": "banned", "subject": { "identity": "erin" } },
  { "namespace": "file", "resource": "design.md", "relation": "parent",
    "subject": { "namespace": "folder", "resource": "specs" } },
  { "namespace": "file", "resource": "design.md", "relation": "owner", "subject": { "identity": "carol" } },
  { "namespace": "file", "resource": "design.md", "relation": "banned", "subject": { "identity": "bob" } },
  { "namespace": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "alice" } },
  { "namespace": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "dave" } }
]
```

So for `file:design.md#viewer`: carol is a viewer through `editor` and `owner`, alice through the folders and group eng, bob is in eng but banned on the file, erin is a viewer of root but banned on specs, and dave is no viewer, so not an auditor either.

A trace: is alice a viewer of file:design.md?

```text
file:design.md#viewer   (this | editor | (parent, viewer)) ! banned
├─ this                → no direct facts                     false
├─ editor              → file:design.md#editor (this | owner) false
└─ (parent, viewer)    → parent fact names folder:specs
   folder:specs#viewer   (this | (parent, viewer)) ! banned
   └─ (parent, viewer) → folder:root
      folder:root#viewer
      └─ this          → subjectset group:eng#member
         group:eng#member → this: alice                       TRUE
      ! folder:root#banned  → false  ⇒ true
   ! folder:specs#banned    → false  ⇒ true
! file:design.md#banned     → false  ⇒ true
```
