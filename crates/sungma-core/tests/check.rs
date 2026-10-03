//! Check against the file/folder theory:
//!
//! ```yaml
//! docs/file:
//!   - owner
//!   - parent
//!   - editor: this | owner
//!   - viewer: (this | editor | (parent, viewer)) ! banned
//!   - auditor: this & viewer
//!   - banned
//! docs/folder:
//!   - owner
//!   - parent
//!   - viewer: (this | (parent, viewer)) ! banned
//!   - banned
//! docs/group:
//!   - member
//! ```

use sungma_core::{
    check::{CheckError, Checker, MAX_DEPTH},
    fixture,
    memory::{MemoryDictionary, MemoryFactStore},
    model::Subject,
    resolve,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
    theory::{Theory, TheoryError},
};

struct World {
    dictionary: MemoryDictionary,
    theory: Theory,
    facts: MemoryFactStore,
}

fn world() -> World {
    let mut dictionary = MemoryDictionary::default();
    let mut theory = Theory::default();
    let viewable = |base: Vec<Rewrite<&'static str>>| {
        Exclusion(Box::new(Union(base)), Box::new(Computed("banned")))
    };
    let parent_viewer = || FactTo {
        factset: "parent",
        computed: "viewer",
    };

    let relations = [
        ("docs/file", "owner", This),
        ("docs/file", "parent", This),
        ("docs/file", "editor", Union(vec![This, Computed("owner")])),
        (
            "docs/file",
            "viewer",
            viewable(vec![This, Computed("editor"), parent_viewer()]),
        ),
        (
            "docs/file",
            "auditor",
            Intersection(vec![This, Computed("viewer")]),
        ),
        ("docs/file", "banned", This),
        ("docs/folder", "owner", This),
        ("docs/folder", "parent", This),
        (
            "docs/folder",
            "viewer",
            viewable(vec![This, parent_viewer()]),
        ),
        ("docs/folder", "banned", This),
        ("docs/group", "member", This),
    ];
    for (namespace, relation, rewrite) in relations {
        fixture::declare(&mut theory, &mut dictionary, namespace, relation, rewrite).unwrap();
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
        theory,
        facts,
    }
}

/// Checks `namespace:id#relation` for an identity, resolving names first.
async fn check(world: &World, set: &str, identity: &str) -> Result<bool, CheckError> {
    let (resource, relation) = set.rsplit_once('#').unwrap();
    let (namespace, id) = resource.split_once(':').unwrap();
    let Some(set) = resolve::subjectset(&world.dictionary, namespace, id, relation).await? else {
        return Ok(false);
    };
    let Some(identity) = resolve::identity(&world.dictionary, identity).await? else {
        return Ok(false);
    };
    Checker::new(&world.theory, &world.facts)
        .check(set, Subject::Identity(identity))
        .await
}

#[tokio::test]
async fn owner_is_editor_and_viewer() {
    let world = world();
    assert!(
        check(&world, "docs/file:design.md#editor", "carol")
            .await
            .unwrap()
    );
    assert!(
        check(&world, "docs/file:design.md#viewer", "carol")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn group_member_views_through_parent_chain() {
    let world = world();
    assert!(
        check(&world, "docs/folder:root#viewer", "alice")
            .await
            .unwrap()
    );
    assert!(
        check(&world, "docs/folder:specs#viewer", "alice")
            .await
            .unwrap()
    );
    assert!(
        check(&world, "docs/file:design.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn banned_is_excluded() {
    let world = world();
    assert!(
        check(&world, "docs/folder:specs#viewer", "bob")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "docs/file:design.md#viewer", "bob")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn ban_on_a_folder_cuts_off_everything_below_it() {
    let world = world();
    assert!(
        check(&world, "docs/folder:root#viewer", "erin")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "docs/folder:specs#viewer", "erin")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "docs/file:design.md#viewer", "erin")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn auditor_must_also_be_viewer() {
    let world = world();
    assert!(
        check(&world, "docs/file:design.md#auditor", "alice")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "docs/file:design.md#auditor", "dave")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn unknown_names_are_denied() {
    let world = world();
    assert!(
        !check(&world, "docs/file:design.md#viewer", "mallory")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "docs/file:nowhere.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn cycle_exceeds_depth() {
    let world = world();
    let result = check(&world, "docs/folder:loop_a#viewer", "alice").await;
    assert!(matches!(result, Err(CheckError::DepthExceeded(MAX_DEPTH))));
}

#[tokio::test]
async fn late_bound_relation_missing_on_target_is_an_error() {
    let world = world();
    let result = check(&world, "docs/file:stray.md#viewer", "alice").await;
    assert!(matches!(result, Err(CheckError::UndeclaredRelation { .. })));
}

#[test]
fn empty_union_is_refused() {
    let mut world = world();
    let result = fixture::declare(
        &mut world.theory,
        &mut world.dictionary,
        "docs/file",
        "odd",
        Union(vec![]),
    );
    assert_eq!(result, Err(TheoryError::EmptyOperator("union")));
}
