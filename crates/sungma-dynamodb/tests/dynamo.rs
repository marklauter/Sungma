//! What only the DynamoDB stores do: writers with clocks of their own on
//! one table, names and leases, the transaction cap, and items Sungma
//! didn't write. Runs against DynamoDB Local, as `stores.rs` does.

mod common;

use std::num::NonZeroU32;

use aws_sdk_dynamodb::{primitives::Blob, types::AttributeValue};
use common::table;
use sungma::{
    graph::{Fact, Resource, Subject, Subjectset},
    id::{NamespaceId, RelationId},
    name::{Identity, ResourceId},
    namespace::Namespace,
    revision::Revision,
    rewrite::Rewrite::This,
    store::{Dictionary, FactReader, NameReader, NamespaceReader, Pool},
};
use sungma_clock::DevClock;
use sungma_dynamodb::{DynamoReader, DynamoWriter, Table, key};
use sungma_write::{
    intern::LeasingInterner,
    store::{FactWrite::Insert, FactWriter, Interner, NameWriter, NamespaceWrite, NamespaceWriter},
};

fn writer(table: &Table) -> DynamoWriter<DevClock> {
    DynamoWriter::new(table.clone(), DevClock::default())
}

const DOC: NamespaceId = NamespaceId(0);
const OWNER: RelationId = RelationId(0);

/// Interns the names and creates `doc` declaring `owner`.
async fn declared(writer: &DynamoWriter<DevClock>) -> Revision {
    writer
        .insert_if_absent(Pool::Namespaces, "doc", DOC.0)
        .await
        .unwrap();
    writer
        .insert_if_absent(Pool::Relations, "owner", OWNER.0)
        .await
        .unwrap();
    let namespace = Namespace::new(vec![(OWNER, This)]).unwrap();
    writer
        .write_namespace(DOC, NamespaceWrite::Create(namespace))
        .await
        .unwrap()
}

fn owner(id: &str, who: &str) -> Fact {
    Fact {
        subjectset: Subjectset {
            resource: Resource {
                namespace: DOC,
                id: ResourceId::new_unchecked(id),
            },
            relation: OWNER,
        },
        subject: Subject::Identity(Identity::new_unchecked(who)),
    }
}

/// A writer whose clock is behind the table's head is refused, moves its
/// clock past the head, and lands its write after every earlier one. A
/// writer opened with `dev` starts after the head.
#[tokio::test]
async fn a_writer_behind_the_head_catches_up_in_one_try() {
    let table = table().await;
    let ahead = writer(&table);
    declared(&ahead).await;
    for _ in 0..5 {
        ahead.write(&[]).await.unwrap();
    }
    let behind = writer(&table);
    let landed = behind.write(&[Insert(owner("a", "alice"))]).await.unwrap();
    assert_eq!(landed, Revision(7));
    let reader = DynamoReader::new(table.clone());
    assert_eq!(reader.head().await.unwrap(), Revision(7));
    let set = owner("a", "alice").subjectset;
    assert_eq!(reader.subjects(&set, Revision(6)).await.unwrap(), []);
    assert_eq!(reader.subjects(&set, Revision(7)).await.unwrap().len(), 1);
    let behind_again = writer(&table);
    let namespace = Namespace::new(vec![(OWNER, This)]).unwrap();
    let based_on = Revision(1);
    let version = behind_again
        .write_namespace(
            DOC,
            NamespaceWrite::NewVersion {
                based_on,
                namespace,
            },
        )
        .await
        .unwrap();
    assert_eq!(version, Revision(8));
    let opened = DynamoWriter::dev(table).await.unwrap();
    assert_eq!(opened.write(&[]).await.unwrap(), Revision(9));
}

/// A transaction holds 100 items; a larger write is refused before it's
/// sent, and stores nothing.
#[tokio::test]
async fn a_write_past_one_transaction_is_refused() {
    let table = table().await;
    let writer = writer(&table);
    declared(&writer).await;
    let writes: Vec<_> = (0..50)
        .map(|i| Insert(owner("a", &format!("p{i}"))))
        .collect();
    let refused = writer.write(&writes).await.unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("more than one transaction holds"),
        "{refused}"
    );
    assert_eq!(DynamoReader::new(table).head().await.unwrap(), Revision(1));
    assert_eq!(writer.write(&writes[..45]).await.unwrap(), Revision(2));
}

#[tokio::test]
async fn names_are_stored_once_and_read_both_ways() {
    let table = table().await;
    let (first, second) = (writer(&table), writer(&table));
    assert_eq!(
        first
            .insert_if_absent(Pool::Relations, "owner", 3)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        second
            .insert_if_absent(Pool::Relations, "owner", 4)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        first
            .insert_if_absent(Pool::Namespaces, "owner", 4)
            .await
            .unwrap(),
        4
    );
    let reader = DynamoReader::new(table);
    assert_eq!(reader.id(Pool::Relations, "owner").await.unwrap(), Some(3));
    assert_eq!(reader.id(Pool::Relations, "viewer").await.unwrap(), None);
    assert_eq!(
        reader.name(Pool::Relations, 3).await.unwrap().as_deref(),
        Some("owner")
    );
    assert_eq!(reader.name(Pool::Relations, 4).await.unwrap(), None);
    assert_eq!(
        reader.name(Pool::Namespaces, 4).await.unwrap().as_deref(),
        Some("owner")
    );
}

/// Leases pass over the counter, and near the end of the ids a lease takes
/// what's left; past the end it's empty.
#[tokio::test]
async fn leases_take_what_is_left_of_the_ids() {
    let table = table().await;
    let writer = writer(&table);
    assert_eq!(writer.lease(Pool::Relations, 10).await.unwrap(), 0..10);
    assert_eq!(writer.lease(Pool::Relations, 5).await.unwrap(), 10..15);
    assert_eq!(writer.lease(Pool::Namespaces, 5).await.unwrap(), 0..5);
    assert_eq!(
        writer.lease(Pool::Relations, u32::MAX).await.unwrap(),
        15..u32::MAX
    );
    assert_eq!(
        writer.lease(Pool::Relations, 1).await.unwrap(),
        u32::MAX..u32::MAX
    );
}

/// The interner mints over the adapter's reader and writer, and Check's
/// dictionary reads what it minted.
#[tokio::test]
async fn an_interner_mints_through_the_adapter() {
    let table = table().await;
    let reader = DynamoReader::new(table.clone());
    let interner =
        LeasingInterner::new(reader.clone(), writer(&table), NonZeroU32::new(4).unwrap());
    let owner = interner.intern(Pool::Relations, "owner").await.unwrap();
    let viewer = interner.intern(Pool::Relations, "viewer").await.unwrap();
    assert_ne!(owner, viewer);
    assert_eq!(
        interner.intern(Pool::Relations, "owner").await.unwrap(),
        owner
    );
    let names = sungma::store::ReadingDictionary(reader);
    assert_eq!(
        names.lookup(Pool::Relations, "viewer").await.unwrap(),
        Some(viewer)
    );
}

fn blob(bytes: Vec<u8>) -> AttributeValue {
    AttributeValue::B(Blob::new(bytes))
}

/// Stores `attributes` at the key, as something other than Sungma might.
async fn put(table: &Table, pk: Vec<u8>, sk: Vec<u8>, attributes: &[(&str, AttributeValue)]) {
    let mut put = table
        .client()
        .put_item()
        .table_name(table.name())
        .item("pk", blob(pk))
        .item("sk", blob(sk));
    for (name, value) in attributes {
        put = put.item(*name, value.clone());
    }
    put.send().await.unwrap();
}

fn number(number: u64) -> AttributeValue {
    AttributeValue::N(number.to_string())
}

/// A store read that finds an item Sungma didn't write fails, naming it.
#[tokio::test]
async fn an_item_sungma_did_not_write_is_refused() {
    let table = table().await;
    let reader = DynamoReader::new(table.clone());
    let (head_pk, head_sk) = key::head();
    put(
        &table,
        head_pk,
        head_sk,
        &[("revision", AttributeValue::S("x".into()))],
    )
    .await;
    let refused = reader.head().await.unwrap_err();
    assert_eq!(refused.0, "item head 0x00 has no valid `revision`");

    let namespace = key::namespace(DOC);
    put(
        &table,
        namespace.clone(),
        key::latest(),
        &[("revision", number(1))],
    )
    .await;
    let refused = reader.version(DOC, Revision(1)).await.unwrap_err();
    assert_eq!(refused.0, "item namespace 0 latest has no valid `document`");

    let entry = |document: &str, epochs: Vec<(&str, AttributeValue)>| {
        let epochs = epochs
            .into_iter()
            .map(|(relation, epoch)| (relation.to_owned(), epoch))
            .collect();
        vec![
            ("revision", number(1)),
            ("document", AttributeValue::S(document.to_owned())),
            ("epochs", AttributeValue::M(epochs)),
        ]
    };
    let cases = [
        (entry("{", vec![]), "a stored namespace document is refused"),
        (
            entry(r#"{"doc":{"owner":"this"}}"#, vec![]),
            "relation 'owner' isn't interned",
        ),
    ];
    for (attributes, refusal) in cases {
        put(&table, namespace.clone(), key::latest(), &attributes).await;
        let refused = reader.version(DOC, Revision(1)).await.unwrap_err();
        assert!(refused.0.starts_with(refusal), "{refused}");
    }
    let writer = writer(&table);
    writer
        .insert_if_absent(Pool::Relations, "owner", 0)
        .await
        .unwrap();
    let document = r#"{"doc":{"owner":"this"}}"#;
    let cases = [
        (vec![("x", number(1))], "epoch of 'x', not a relation id"),
        (
            vec![("0", AttributeValue::S("1".into()))],
            "epoch of 0 isn't a number",
        ),
        (
            vec![("0", AttributeValue::N("-1".into()))],
            "epoch of 0 isn't a revision",
        ),
        (
            vec![],
            "relation RelationId(0) is declared but has no epoch",
        ),
    ];
    for (epochs, refusal) in cases {
        put(
            &table,
            namespace.clone(),
            key::latest(),
            &entry(document, epochs),
        )
        .await;
        let refused = reader.version(DOC, Revision(1)).await.unwrap_err();
        assert_eq!(refused.0, refusal);
    }

    let fact = owner("a", "alice");
    put(
        &table,
        key::fact(&fact.subjectset),
        vec![9],
        &[("written", number(1))],
    )
    .await;
    let refused = reader
        .subjects(&fact.subjectset, Revision(1))
        .await
        .unwrap_err();
    assert_eq!(refused.0, "malformed key fact 0:a#0 0x09");
    let (names, id) = (key::names(Pool::Relations), key::name("bad"));
    put(&table, names, id, &[("id", number(u64::MAX))]).await;
    let refused = reader.id(Pool::Relations, "bad").await.unwrap_err();
    assert_eq!(refused.0, format!("id {} doesn't fit in 32 bits", u64::MAX));
}

/// A namespace write needs a name for the namespace and each relation, to
/// print its document.
#[tokio::test]
async fn a_namespace_without_names_is_refused() {
    let table = table().await;
    let writer = writer(&table);
    let namespace = || Namespace::new(vec![(OWNER, This)]).unwrap();
    let refused = writer
        .write_namespace(DOC, NamespaceWrite::Create(namespace()))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "store failure: namespace 0 has no interned name"
    );
    writer
        .insert_if_absent(Pool::Namespaces, "doc", 0)
        .await
        .unwrap();
    let refused = writer
        .write_namespace(DOC, NamespaceWrite::Create(namespace()))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "store failure: relation 0 has no interned name"
    );
}

/// Writers on separate nodes, each with its own clock, all based on one
/// version, race for it: the table's conditions let exactly one through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_nodes_writing_one_version_exactly_one_succeeds() {
    for _ in 0..10 {
        let table = table().await;
        let first = declared(&writer(&table)).await;
        let racers: Vec<_> = (0..4)
            .map(|_| {
                let node = writer(&table);
                tokio::spawn(async move {
                    let namespace = Namespace::new(vec![(OWNER, This)]).unwrap();
                    let write = NamespaceWrite::NewVersion {
                        based_on: first,
                        namespace,
                    };
                    node.write_namespace(DOC, write).await.is_ok()
                })
            })
            .collect();
        let mut stored = 0;
        for racer in racers {
            stored += usize::from(racer.await.unwrap());
        }
        assert_eq!(stored, 1);
    }
}

/// A fact write is judged by the namespace versions current when it lands,
/// whatever its clock: an insert under a relation that another node drops
/// at the same time lands before the drop, or is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_insert_racing_a_drop_lands_before_it_or_not_at_all() {
    const OTHER: RelationId = RelationId(1);
    for _ in 0..20 {
        let table = table().await;
        let first = declared(&writer(&table)).await;
        let names = writer(&table);
        names
            .insert_if_absent(Pool::Relations, "other", OTHER.0)
            .await
            .unwrap();
        // Far ahead, so the head never refuses the insert.
        let facts = DynamoWriter::new(table.clone(), DevClock::starting_after(Revision(1000)));
        let namespaces = writer(&table);
        // Forty facts, so the insert reads forty pointers between reading
        // the namespace and landing, and the drop can land in between.
        let grants: Vec<_> = (0..40)
            .map(|i| Insert(owner("a", &format!("p{i}"))))
            .collect();
        let insert = tokio::spawn(async move { facts.write(&grants).await });
        let drop = tokio::spawn(async move {
            let namespace = Namespace::new(vec![(OTHER, This)]).unwrap();
            let write = NamespaceWrite::NewVersion {
                based_on: first,
                namespace,
            };
            namespaces.write_namespace(DOC, write).await.unwrap()
        });
        let (inserted, dropped) = (insert.await.unwrap(), drop.await.unwrap());
        let reader = DynamoReader::new(table);
        let current = reader.version(DOC, Revision(2000)).await.unwrap().unwrap();
        assert_eq!(current.revision(), dropped);
        match inserted {
            Ok(revision) => {
                let at_insert = reader.version(DOC, revision).await.unwrap().unwrap();
                assert!(
                    at_insert.declares(OWNER),
                    "{revision:?} landed after the drop at {dropped:?}"
                );
            }
            Err(refused) => assert!(
                matches!(refused, sungma_write::store::FactWriteError::Undeclared(_)),
                "{refused}"
            ),
        }
    }
}
