# Interner

How `LeasingInterner` mints an id for a name. Each node runs its own interner over the shared names, which it reads through a `NameReader` and writes through a `NameWriter`.

## Which names are interned

Namespace and relation names only, in `Pool::Namespaces` and `Pool::Relations`. Resource ids and identities stay strings in facts.

- Namespace and relation names are few and bounded, and change only with a namespace version, so every node can cache the whole dictionary.
- Resource ids and identities are unbounded and grow with the fact store. Interning them would make the dictionary a second large store: replicated to every node, read on every check, written in revision order with the facts so a name resolves at least as fresh as the zookie, and garbage-collected as facts go. A `u32` id would also cap them at about four billion.

## Minting an id

```mermaid
sequenceDiagram
    participant C as Caller
    participant I as LeasingInterner (node)
    participant L as Leases (per pool, in memory)
    participant R as NameReader (shared)
    participant W as NameWriter (shared)

    C->>I: intern(pool, name)
    I->>R: id(pool, name)
    alt name already stored
        R-->>I: Some(id)
        I-->>C: id
    else name is new
        R-->>I: None
        I->>L: take(pool)
        alt block has ids left
            L-->>I: Some(id)
        else block spent (or none yet)
            L-->>I: None
            I->>W: lease(pool, block)
            Note over W: counter advances atomically,<br/>no other node gets this range
            W-->>I: start..end
            alt range empty
                I-->>C: Err(no ids left)
            else
                I->>L: store rest of block for pool
            end
        end
        I->>W: insert_if_absent(pool, name, id)
        alt this node stored first
            W-->>I: id
        else another node won the race
            Note over W: name already maps to winner's id,<br/>this node's id becomes a gap
            W-->>I: winner's id
        end
        I-->>C: stored id
    end
```

The store is touched in three places, each a single atomic operation:

1. `id` is the fast path; every name after its first use ends here.
2. `lease` reaches the shared counter once per block, not once per name.
3. `insert_if_absent` is the conditional insert that settles a race to whichever id was stored first.
