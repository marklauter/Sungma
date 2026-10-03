//! Contains and Expand against the theories in [`common`].

mod common;

use common::{World, head, identity, subjectset, world};
use sungma_core::{
    extent::{Expansion, Extent, ExtentError, MAX_DEPTH},
    fixture,
    model::{Revision, Subject},
    rewrite::Rewrite::Union,
    theory::TheoryError,
};

/// Whether an identity is in the extent of `theory:id#relation` at `revision`.
async fn check_at(
    world: &World,
    set: &str,
    who: &str,
    revision: Revision,
) -> Result<bool, ExtentError> {
    let (Some(set), Some(subject)) = (subjectset(world, set).await, identity(world, who).await)
    else {
        return Ok(false);
    };
    Extent::new(&world.theories, &world.facts, set, revision)
        .contains(subject)
        .await
}

/// [`check_at`] the latest revision.
async fn check(world: &World, set: &str, who: &str) -> Result<bool, ExtentError> {
    check_at(world, set, who, head(world).await).await
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
async fn fact_to_subjectset_follows_the_resource_of_a_stored_subjectset() {
    let world = world();
    assert!(
        check(&world, "file:notes.md#viewer", "alice")
            .await
            .unwrap()
    );
    assert!(
        !check(&world, "file:notes.md#viewer", "frank")
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
    assert!(matches!(result, Err(ExtentError::DepthExceeded(MAX_DEPTH))));
}

#[tokio::test]
async fn late_bound_relation_missing_on_target_is_empty() {
    let world = world();
    assert!(
        !check(&world, "file:stray.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn expand_of_an_undeclared_relation_is_none() {
    let world = world();
    let viewer = subjectset(&world, "group:eng#viewer").await.unwrap();
    let expansion = Extent::new(&world.theories, &world.facts, viewer, head(&world).await)
        .expand()
        .await
        .unwrap();
    assert_eq!(expansion, None);
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

#[tokio::test]
async fn facts_written_after_the_pin_are_not_visible() {
    let mut world = world();
    let before = head(&world).await;
    let fact = r#"[{ "set": "file:design.md#viewer", "identity": "zed" }]"#;
    fixture::load_facts(fact, &mut world.dictionary, &mut world.facts).unwrap();
    assert!(
        !check_at(&world, "file:design.md#viewer", "zed", before)
            .await
            .unwrap()
    );
    assert!(check(&world, "file:design.md#viewer", "zed").await.unwrap());
}

#[tokio::test]
async fn expand_leaves_referenced_subjectsets_as_leaves() {
    let world = world();
    let set = |text| subjectset(&world, text);
    let viewer = set("file:design.md#viewer").await.unwrap();
    let expansion = Extent::new(&world.theories, &world.facts, viewer, head(&world).await)
        .expand()
        .await
        .unwrap()
        .unwrap();
    let expected = Expansion::Exclusion(
        Box::new(Expansion::Union(vec![
            Expansion::Subjects(vec![]),
            Expansion::Reference(set("file:design.md#editor").await.unwrap()),
            Expansion::Union(vec![Expansion::Reference(
                set("folder:specs#viewer").await.unwrap(),
            )]),
        ])),
        Box::new(Expansion::Reference(
            set("file:design.md#banned").await.unwrap(),
        )),
    );
    assert_eq!(expansion, expected);
}

#[tokio::test]
async fn expand_lists_direct_subjects_including_subjectsets() {
    let world = world();
    let root_viewer = subjectset(&world, "folder:root#viewer").await.unwrap();
    let expansion = Extent::new(
        &world.theories,
        &world.facts,
        root_viewer,
        head(&world).await,
    )
    .expand()
    .await
    .unwrap();
    let Some(Expansion::Exclusion(base, _)) = expansion else {
        panic!("viewer is an exclusion");
    };
    let Expansion::Union(operands) = *base else {
        panic!("its base is a union");
    };
    let eng = subjectset(&world, "group:eng#member").await.unwrap();
    let erin = identity(&world, "erin").await.unwrap();
    assert_eq!(
        operands[0],
        Expansion::Subjects(vec![erin, Subject::Subjectset(eng)])
    );
}
