//! Check against the file, folder and group theories:
//!
//! ```yaml
//! file:
//!   - owner
//!   - parent
//!   - editor: this | owner
//!   - viewer: (this | editor | (parent, viewer)) ! banned
//!   - auditor: this & viewer
//!   - banned
//! folder:
//!   - owner
//!   - parent
//!   - viewer: (this | (parent, viewer)) ! banned
//!   - banned
//! group:
//!   - member
//! ```

use sungma_core::{
    check::{CheckError, Checker, MAX_DEPTH},
    fixture,
    memory::{MemoryDictionary, MemoryFactStore},
    model::Subject,
    resolve,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
    theory::{Theories, TheoryError},
};

struct World {
    dictionary: MemoryDictionary,
    theories: Theories,
    facts: MemoryFactStore,
}

fn world() -> World {
    let mut dictionary = MemoryDictionary::default();
    let mut theories = Theories::default();
    let viewable = |base: Vec<Rewrite<&'static str>>| {
        Exclusion(Box::new(Union(base)), Box::new(Computed("banned")))
    };
    let parent_viewer = || FactTo {
        factset: "parent",
        computed: "viewer",
    };

    let relations = [
        ("file", "owner", This),
        ("file", "parent", This),
        ("file", "editor", Union(vec![This, Computed("owner")])),
        (
            "file",
            "viewer",
            viewable(vec![This, Computed("editor"), parent_viewer()]),
        ),
        (
            "file",
            "auditor",
            Intersection(vec![This, Computed("viewer")]),
        ),
        ("file", "banned", This),
        ("folder", "owner", This),
        ("folder", "parent", This),
        ("folder", "viewer", viewable(vec![This, parent_viewer()])),
        ("folder", "banned", This),
        ("group", "member", This),
    ];
    for (theory, relation, rewrite) in relations {
        fixture::declare(&mut theories, &mut dictionary, theory, relation, rewrite).unwrap();
    }

    let mut facts = MemoryFactStore::default();
    fixture::load_facts(
        include_str!("fixtures/docs.json"),
        &mut dictionary,
        &mut facts,
    )
    .unwrap();
    World {
        dictionary,
        theories,
        facts,
    }
}

/// Checks `theory:id#relation` for an identity, resolving names first.
async fn check(world: &World, set: &str, identity: &str) -> Result<bool, CheckError> {
    let (resource, relation) = set.rsplit_once('#').unwrap();
    let (theory, id) = resource.split_once(':').unwrap();
    let Some(set) = resolve::subjectset(&world.dictionary, theory, id, relation).await? else {
        return Ok(false);
    };
    let Some(identity) = resolve::identity(&world.dictionary, identity).await? else {
        return Ok(false);
    };
    Checker::new(&world.theories, &world.facts)
        .check(set, Subject::Identity(identity))
        .await
}

#[tokio::test]
async fn owner_is_editor_and_viewer() {
    let world = world();
    assert!(
        check(&world, "file:design.md#editor", "carol")
            .await
            .unwrap()
    );
    assert!(
        check(&world, "file:design.md#viewer", "carol")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn group_member_views_through_parent_chain() {
    let world = world();
    assert!(check(&world, "folder:root#viewer", "alice").await.unwrap());
    assert!(check(&world, "folder:specs#viewer", "alice").await.unwrap());
    assert!(
        check(&world, "file:design.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn banned_is_excluded() {
    let world = world();
    assert!(check(&world, "folder:specs#viewer", "bob").await.unwrap());
    assert!(!check(&world, "file:design.md#viewer", "bob").await.unwrap());
}

#[tokio::test]
async fn ban_on_a_folder_cuts_off_everything_below_it() {
    let world = world();
    assert!(check(&world, "folder:root#viewer", "erin").await.unwrap());
    assert!(!check(&world, "folder:specs#viewer", "erin").await.unwrap());
    assert!(
        !check(&world, "file:design.md#viewer", "erin")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn auditor_must_also_be_viewer() {
    let world = world();
    assert!(
        check(&world, "file:design.md#auditor", "alice")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "file:design.md#auditor", "dave")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn unknown_names_are_denied() {
    let world = world();
    assert!(
        !check(&world, "file:design.md#viewer", "mallory")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "file:nowhere.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn cycle_exceeds_depth() {
    let world = world();
    let result = check(&world, "folder:loop_a#viewer", "alice").await;
    assert!(matches!(result, Err(CheckError::DepthExceeded(MAX_DEPTH))));
}

#[tokio::test]
async fn late_bound_relation_missing_on_target_is_an_error() {
    let world = world();
    let result = check(&world, "file:stray.md#viewer", "alice").await;
    assert!(matches!(result, Err(CheckError::UndeclaredRelation { .. })));
}

#[test]
fn empty_union_is_refused() {
    let mut world = world();
    let result = fixture::declare(
        &mut world.theories,
        &mut world.dictionary,
        "file",
        "odd",
        Union(vec![]),
    );
    assert_eq!(result, Err(TheoryError::EmptyOperator("union")));
}
