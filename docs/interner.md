# Interner

How `LeasingInterner` mints an id for a name. Each node runs its own interner over the shared `NameStore`.

## Which names are interned

Theory and relation names only, in `Pool::Theories` and `Pool::Relations`. Resource ids and identities stay strings in facts.

- Theory and relation names are few and bounded, and change only with a theory version, so every node can cache the whole dictionary.
- Resource ids and identities are unbounded and grow with the fact store. Interning them would make the dictionary a second large store: replicated to every node, read on every check, written in revision order with the facts so a name resolves at least as fresh as the zookie, and garbage-collected as facts go. A `u32` id would also cap them at about four billion.

## Minting an id

```mermaid
sequenceDiagram
    participant C as Caller
    participant I as LeasingInterner (node)
    participant L as Leases (per pool, in memory)
    participant S as NameStore (shared)

    C->>I: intern(pool, name)
    I->>S: lookup(pool, name)
    alt name already stored
        S-->>I: Some(id)
        I-->>C: id
    else name is new
        S-->>I: None
        I->>L: take(pool)
        alt block has ids left
            L-->>I: Some(id)
        else block spent (or none yet)
            L-->>I: None
            I->>S: lease(pool, block)
            Note over S: counter advances atomically,<br/>no other node gets this range
            S-->>I: start..end
            alt range empty
                I-->>C: Err(no ids left)
            else
                I->>L: store rest of block for pool
            end
        end
        I->>S: insert_if_absent(pool, name, id)
        alt this node stored first
            S-->>I: id
        else another node won the race
            Note over S: name already maps to winner's id,<br/>this node's id becomes a gap
            S-->>I: winner's id
        end
        I-->>C: stored id
    end
```

The store is touched in three places, each a single atomic operation:

1. `lookup` is the fast path; every name after its first use ends here.
2. `lease` reaches the shared counter once per block, not once per name.
3. `insert_if_absent` is the conditional insert that settles a race to whichever id was stored first.
