//! Contains and Expand against the theories in [`common`].

mod common;

use common::{World, head, identity, subjectset, world};
use sungma_core::{
    extent::{Expansion, Extent, ExtentError, MAX_DEPTH},
    fixture::{self, FixtureError},
    model::{Revision, Subject},
    rewrite::Rewrite::{Intersection, Union},
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

/// The docs world plus cycles of stored subjectsets. `ring_a` and `ring_b`
/// contain each other and nobody else. `cyc_a` and `cyc_b` contain each
/// other, and `cyc_a` also reaches alice through `cyc_c`; `cyc_b` is
/// interned first, so the cyclic nested set is read before `cyc_c`.
fn ringed_world() -> World {
    let mut world = world();
    let facts = r#"[
        { "set": "group:ring_a#member", "subjectset": "group:ring_b#member" },
        { "set": "group:ring_b#member", "subjectset": "group:ring_a#member" },
        { "set": "group:cyc_a#member", "subjectset": "group:cyc_b#member" },
        { "set": "group:cyc_b#member", "subjectset": "group:cyc_a#member" },
        { "set": "group:cyc_a#member", "subjectset": "group:cyc_c#member" },
        { "set": "group:cyc_c#member", "identity": "alice" },
        { "set": "folder:open#viewer", "identity": "alice" },
        { "set": "folder:open#banned", "subjectset": "group:ring_a#member" },
        { "set": "file:ring.md#viewer", "subjectset": "group:ring_a#member" },
        { "set": "file:ring.md#owner", "identity": "alice" }
    ]"#;
    fixture::load_facts(facts, &mut world.dictionary, &mut world.facts).unwrap();
    world
}

#[tokio::test]
async fn a_cycle_of_parents_denies_a_non_member() {
    let world = world();
    let result = check(&world, "folder:loop_a#viewer", "alice").await;
    assert!(!result.unwrap());
}

#[tokio::test]
async fn a_cycle_of_subjectsets_denies_a_non_member() {
    let world = ringed_world();
    assert!(!check(&world, "group:ring_a#member", "alice").await.unwrap());
    assert!(!check(&world, "group:cyc_a#member", "bob").await.unwrap());
}

#[tokio::test]
async fn a_cyclic_nested_set_does_not_hide_a_later_one() {
    let world = ringed_world();
    assert!(check(&world, "group:cyc_a#member", "alice").await.unwrap());
    assert!(check(&world, "group:cyc_b#member", "alice").await.unwrap());
}

#[tokio::test]
async fn a_cyclic_union_operand_does_not_hide_a_later_one() {
    let world = ringed_world();
    // this cycles through ring_a; editor grants through owner.
    assert!(check(&world, "file:ring.md#viewer", "alice").await.unwrap());
}

#[tokio::test]
async fn a_cycle_under_an_exclusion_excludes_nobody() {
    let world = ringed_world();
    assert!(check(&world, "folder:open#viewer", "alice").await.unwrap());
}

#[tokio::test]
async fn a_long_acyclic_chain_exceeds_depth() {
    let mut world = world();
    let chain: Vec<String> = (0..=MAX_DEPTH)
        .map(|i| {
            format!(
                r#"{{ "set": "group:g{i}#member", "subjectset": "group:g{}#member" }}"#,
                i + 1
            )
        })
        .collect();
    let facts = format!("[{}]", chain.join(","));
    fixture::load_facts(&facts, &mut world.dictionary, &mut world.facts).unwrap();
    let result = check(&world, "group:g0#member", "alice").await;
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

#[test]
fn empty_intersection_is_refused() {
    let mut world = world();
    let result = fixture::declare(
        &mut world.theories,
        &mut world.dictionary,
        "file",
        "odd",
        Intersection(vec![]),
    );
    assert_eq!(result, Err(TheoryError::EmptyOperator("intersection")));
}

#[test]
fn theory_json_refuses_an_empty_operator() {
    let mut world = world();
    let json = r#"{ "file": { "odd": { "union": [] } } }"#;
    let result = fixture::load_theories(json, &mut world.dictionary, &mut world.theories);
    assert!(matches!(
        result,
        Err(FixtureError::Theory(TheoryError::EmptyOperator("union")))
    ));
}

#[tokio::test]
async fn expand_keeps_an_intersection() {
    let world = world();
    let auditor = subjectset(&world, "file:design.md#auditor").await.unwrap();
    let expansion = Extent::new(&world.theories, &world.facts, auditor, head(&world).await)
        .expand()
        .await
        .unwrap();
    let alice = identity(&world, "alice").await.unwrap();
    let dave = identity(&world, "dave").await.unwrap();
    let viewer = subjectset(&world, "file:design.md#viewer").await.unwrap();
    let expected = Expansion::Intersection(vec![
        Expansion::Subjects(vec![alice, dave]),
        Expansion::Reference(viewer),
    ]);
    assert_eq!(expansion, Some(expected));
}

#[tokio::test]
async fn facts_written_after_the_revision_are_not_visible() {
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
