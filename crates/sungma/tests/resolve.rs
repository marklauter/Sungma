//! Resolving names to ids through a dictionary, and back.

use sungma::{
    graph::Fact,
    id::{NamespaceId, RelationId},
    name::{FactName, NamespaceName, RelationName, SubjectName},
    namespace::Namespace,
    rewrite::Rewrite,
};
use sungma_edge::memory::MemorySequence;
use sungma_names::store::NameWriter;
use sungma_names::write::NameRowWriter;
use sungma_names::{
    resolve::{self, ResolveError, UnknownName},
    store::Pool,
};
use sungma_node::testkit::World;

/// `file`, `folder` and `group`, and the relations `owner`, `parent`,
/// `member` and `viewer`.
async fn world() -> World {
    let world = World::new();
    for namespace in ["file", "folder", "group"] {
        world.intern(Pool::Namespaces, namespace).await;
    }
    for relation in ["owner", "parent", "member", "viewer"] {
        world.intern(Pool::Relations, relation).await;
    }
    world
}

fn fact(subjectset: &str, subject: SubjectName) -> FactName {
    FactName {
        subjectset: subjectset.parse().unwrap(),
        subject,
    }
}

fn subjects() -> [SubjectName; 3] {
    [
        SubjectName::Identity("alice".parse().unwrap()),
        SubjectName::Resource("folder:root".parse().unwrap()),
        SubjectName::Subjectset("group:eng#member".parse().unwrap()),
    ]
}

#[tokio::test]
async fn a_fact_resolves_to_ids_and_back_to_its_names() {
    let world = world().await;
    let dictionary = world.names();
    for subject in subjects() {
        let name = fact("file:readme#viewer", subject);
        let Fact {
            subjectset,
            subject,
        } = resolve::fact(dictionary, &name).await.unwrap();
        assert_eq!(
            resolve::subjectset_name(dictionary, &subjectset)
                .await
                .unwrap(),
            name.subjectset
        );
        assert_eq!(
            resolve::subject_name(dictionary, &subject).await.unwrap(),
            name.subject
        );
    }
}

#[tokio::test]
async fn a_name_never_interned_is_unknown() {
    let world = world().await;
    let dictionary = world.names();
    let cases = [
        (
            fact("user:a#owner", subjects()[0].clone()),
            UnknownName::Namespace("user".parse().unwrap()),
        ),
        (
            fact("file:a#editor", subjects()[0].clone()),
            UnknownName::Relation("editor".parse().unwrap()),
        ),
        (
            fact(
                "file:a#owner",
                SubjectName::Resource("user:a".parse().unwrap()),
            ),
            UnknownName::Namespace("user".parse().unwrap()),
        ),
        (
            fact(
                "file:a#owner",
                SubjectName::Subjectset("group:eng#admin".parse().unwrap()),
            ),
            UnknownName::Relation("admin".parse().unwrap()),
        ),
    ];
    for (name, unknown) in cases {
        let resolved = resolve::fact(dictionary, &name).await;
        assert!(
            matches!(&resolved, Err(ResolveError::Unknown(found)) if *found == unknown),
            "{name}: {resolved:?}"
        );
        assert_eq!(resolve::known(resolved).unwrap(), None, "{name}");
    }
}

#[tokio::test]
async fn an_id_with_no_name_or_a_stored_non_name_fails_as_the_store() {
    let world = world().await;
    let edge = world.node().edge();
    NameRowWriter::new(edge, edge, MemorySequence::new())
        .insert_if_absent(Pool::Relations, "Not a name!", 1000)
        .await
        .unwrap();
    let dictionary = world.names();
    let missing = resolve::namespace_name(dictionary, NamespaceId(999)).await;
    assert!(missing.unwrap_err().0.contains("999"));
    let malformed = resolve::relation_name(dictionary, RelationId(1000)).await;
    assert!(malformed.unwrap_err().0.contains("Not a name!"));
}

#[tokio::test]
async fn a_store_failure_is_not_an_unknown_name() {
    let resolved: Result<(), ResolveError> =
        Err(sungma::store::StoreError("down".to_owned()).into());
    assert!(resolve::known(resolved).is_err());
}

#[tokio::test]
async fn a_namespace_gets_its_relation_names_back() {
    let world = world().await;
    let dictionary = world.names();
    let named = |name: &str| name.parse::<RelationName>().unwrap();
    let namespace = Namespace::new(vec![
        (named("owner"), Rewrite::This),
        (named("parent"), Rewrite::This),
        (
            named("viewer"),
            Rewrite::Union(sungma::nonempty![
                Rewrite::Computed(named("owner")),
                Rewrite::FactTo {
                    factset: named("parent"),
                    computed: named("member"),
                },
            ]),
        ),
    ])
    .unwrap();
    let mut ids = Vec::new();
    for name in namespace.names() {
        ids.push((
            name.clone(),
            resolve::relation(dictionary, name).await.unwrap(),
        ));
    }
    let interned = namespace
        .map(|name| ids.iter().find(|(n, _)| n == name).unwrap().1)
        .unwrap();
    let back = resolve::namespace_names(dictionary, &interned)
        .await
        .unwrap();
    let relations = |namespace: &Namespace<RelationName>| {
        namespace
            .relations()
            .map(|(name, rewrite)| (name.clone(), rewrite.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(relations(&back), relations(&namespace));
    let resolved = resolve::namespace(dictionary, &"file".parse::<NamespaceName>().unwrap()).await;
    assert!(resolved.is_ok());
}

#[test]
fn a_namespace_names_each_relation_once_in_the_order_met() {
    let named = |name: &str| name.parse::<RelationName>().unwrap();
    let namespace = Namespace::new(vec![
        (
            named("viewer"),
            Rewrite::Exclusion(
                Box::new(Rewrite::Intersection(sungma::nonempty![
                    Rewrite::Computed(named("owner")),
                    Rewrite::FactTo {
                        factset: named("parent"),
                        computed: named("viewer"),
                    },
                ])),
                Box::new(Rewrite::Computed(named("banned"))),
            ),
        ),
        (named("owner"), Rewrite::This),
        (named("parent"), Rewrite::This),
        (named("banned"), Rewrite::This),
    ])
    .unwrap();
    let names: Vec<&str> = namespace.names().iter().map(|name| name.as_str()).collect();
    assert_eq!(names, ["viewer", "owner", "parent", "banned"]);
}

#[test]
fn a_fact_name_reads_as_the_string_form() {
    let [identity, resource, subjectset] = subjects();
    for (subject, text) in [
        (identity, "file:readme#viewer@alice"),
        (resource, "file:readme#viewer@folder:root"),
        (subjectset, "file:readme#viewer@group:eng#member"),
    ] {
        assert_eq!(fact("file:readme#viewer", subject).to_string(), text);
    }
}
