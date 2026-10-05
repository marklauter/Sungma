//! Contains and Expand against the theories in [`common`].

mod common;

use std::sync::Arc;

use common::{World, head, identity, subjectset, world};
use proptest::{prelude::*, sample::subsequence, test_runner::TestCaseError};
use sungma_core::{
    extent::{Expansion, Extent, ExtentError, MAX_DEPTH},
    fixture::{self, FixtureError},
    model::{RelationId, Revision, Subject, Subjectset, TheoryId},
    rewrite::Rewrite::{self, Computed, Exclusion, Intersection, This, Union},
    store::{Dictionary, FactStore, Pool, StoreError, TheoryStore},
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
    agree(
        &Extent::new(&world.theories, &world.facts, set, revision),
        subject,
    )
    .await
}

/// [`Extent::contains`], checked against [`Extent::decide`]: the same
/// verdict, or both fail.
async fn agree<T: TheoryStore + Sync, F: FactStore + Sync>(
    extent: &Extent<'_, T, F>,
    subject: Subject,
) -> Result<bool, ExtentError> {
    let fast = extent.contains(subject).await;
    let decided = extent.decide(subject).await;
    match (&fast, &decided) {
        (Ok(fast), Ok(decided)) => assert_eq!(*fast, decided.outcome.is_allowed()),
        (Err(_), Err(_)) => {}
        _ => panic!("contains {fast:?} and decide {decided:?} disagree"),
    }
    fast
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
    let world = world();
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
    fixture::load_facts(facts, &world.dictionary, &world.facts).unwrap();
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

/// Checks `group:g0#member` down a chain of `length` subjectsets.
async fn chain(length: usize) -> Result<bool, ExtentError> {
    let world = world();
    let chain: Vec<String> = (1..length)
        .map(|i| {
            format!(
                r#"{{ "set": "group:g{}#member", "subjectset": "group:g{i}#member" }}"#,
                i - 1
            )
        })
        .collect();
    let facts = format!("[{}]", chain.join(","));
    fixture::load_facts(&facts, &world.dictionary, &world.facts).unwrap();
    check(&world, "group:g0#member", "alice").await
}

#[tokio::test]
async fn a_chain_may_be_max_depth_long() {
    assert!(!chain(MAX_DEPTH).await.unwrap());
}

#[tokio::test]
async fn a_longer_chain_exceeds_depth() {
    let result = chain(MAX_DEPTH + 1).await;
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
async fn a_parent_of_an_undeclared_theory_is_empty() {
    let world = world();
    let fact = r#"[{ "set": "file:haunted.md#parent", "resource": "ghost:attic" }]"#;
    fixture::load_facts(fact, &world.dictionary, &world.facts).unwrap();
    assert!(
        !check(&world, "file:haunted.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Read {
    Contains,
    Subjectsets,
    Subjects,
    /// The rewrite of the subjectset's theory and relation, for any resource.
    Rewrite,
}

/// The docs theory and fact stores, failing the listed reads. With a seed,
/// the operands of every union and intersection come back shuffled.
struct Probe<'a> {
    world: &'a World,
    failures: Vec<(Subjectset, Read)>,
    shuffle: Option<u64>,
}

impl<'a> Probe<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world,
            failures: Vec::new(),
            shuffle: None,
        }
    }

    fn fail(&self, set: Subjectset, read: Read) -> Result<(), StoreError> {
        if self.failures.contains(&(set, read)) {
            return Err(StoreError("injected".to_owned()));
        }
        Ok(())
    }
}

/// `rewrite` with the operands of each union and intersection permuted by
/// `seed`. An exclusion's sides keep their places.
fn shuffled(rewrite: &Rewrite, seed: &mut u64) -> Rewrite {
    let mut permute = |operands: &[Rewrite]| {
        let mut operands: Vec<Rewrite> = operands.iter().map(|op| shuffled(op, seed)).collect();
        for i in (1..operands.len()).rev() {
            *seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            operands.swap(i, (*seed >> 33) as usize % (i + 1));
        }
        operands
    };
    match rewrite {
        Union(operands) => Union(permute(operands)),
        Intersection(operands) => Intersection(permute(operands)),
        Exclusion(base, excluded) => Exclusion(
            Box::new(shuffled(base, seed)),
            Box::new(shuffled(excluded, seed)),
        ),
        leaf => leaf.clone(),
    }
}

impl TheoryStore for Probe<'_> {
    async fn rewrite(
        &self,
        theory: TheoryId,
        relation: RelationId,
    ) -> Result<Option<Arc<Rewrite>>, StoreError> {
        let failed = self.failures.iter().any(|&(set, read)| {
            read == Read::Rewrite && (set.resource.theory, set.relation) == (theory, relation)
        });
        if failed {
            return Err(StoreError("injected".to_owned()));
        }
        let rewrite = self.world.theories.rewrite(theory, relation).await?;
        Ok(match self.shuffle {
            Some(mut seed) => rewrite.map(|rewrite| Arc::new(shuffled(&rewrite, &mut seed))),
            None => rewrite,
        })
    }
}

impl FactStore for Probe<'_> {
    async fn head(&self) -> Result<Revision, StoreError> {
        self.world.facts.head().await
    }

    async fn contains(
        &self,
        set: Subjectset,
        subject: Subject,
        revision: Revision,
    ) -> Result<bool, StoreError> {
        self.fail(set, Read::Contains)?;
        self.world.facts.contains(set, subject, revision).await
    }

    async fn subjectsets(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subjectset>, StoreError> {
        self.fail(set, Read::Subjectsets)?;
        self.world.facts.subjectsets(set, revision).await
    }

    async fn subjects(
        &self,
        set: Subjectset,
        revision: Revision,
    ) -> Result<Vec<Subject>, StoreError> {
        self.fail(set, Read::Subjects)?;
        self.world.facts.subjects(set, revision).await
    }
}

/// An [`Extent`] over `checked` whose stores fail `read` of `failed`.
async fn failing<'a>(
    world: &'a World,
    store: &'a mut Option<Probe<'a>>,
    checked: &str,
    failed: &str,
    read: Read,
) -> Extent<'a, Probe<'a>, Probe<'a>> {
    let set = subjectset(world, failed).await.unwrap();
    let store = store.insert(Probe {
        failures: vec![(set, read)],
        ..Probe::new(world)
    });
    let checked = subjectset(world, checked).await.unwrap();
    Extent::new(store, store, checked, head(world).await)
}

/// Checks `who` against `checked` with `read` of `failed` failing.
async fn check_failing(
    world: &World,
    checked: &str,
    who: &str,
    failed: &str,
    read: Read,
) -> Result<bool, ExtentError> {
    let who = identity(world, who).await.unwrap();
    let mut store = None;
    let extent = failing(world, &mut store, checked, failed, read).await;
    agree(&extent, who).await
}

#[tokio::test]
async fn a_store_error_that_nothing_outweighs_fails_the_check() {
    let world = world();
    let cases = [
        // Every viewer operand is false or failed.
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#parent",
            Read::Subjects,
        ),
        // The only path to alice fails.
        (
            "file:design.md#viewer",
            "alice",
            "group:eng#member",
            Read::Contains,
        ),
        (
            "file:design.md#viewer",
            "alice",
            "folder:specs#viewer",
            Read::Rewrite,
        ),
        // The base holds, and whether alice is banned is unknown.
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#banned",
            Read::Contains,
        ),
        // alice is stored as an auditor, and whether she views is unknown.
        (
            "file:design.md#auditor",
            "alice",
            "group:eng#member",
            Read::Contains,
        ),
    ];
    for (checked, who, failed, read) in cases {
        let result = check_failing(&world, checked, who, failed, read).await;
        assert!(
            matches!(result, Err(ExtentError::Store(_))),
            "{checked} for {who} failing on {failed}: {result:?}"
        );
    }
}

#[tokio::test]
async fn an_operand_that_settles_the_check_outweighs_a_store_error() {
    let world = world();
    let facts = r#"[
        { "set": "file:twin.md#parent", "resource": "folder:specs" },
        { "set": "file:twin.md#parent", "resource": "folder:shared" },
        { "set": "folder:shared#viewer", "identity": "alice" }
    ]"#;
    fixture::load_facts(facts, &world.dictionary, &world.facts).unwrap();
    let cases = [
        // A union's later operand holds: alice views through the parent.
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#viewer",
            Read::Contains,
            true,
        ),
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#viewer",
            Read::Subjectsets,
            true,
        ),
        // A stored subjectset holds after the point lookup fails.
        (
            "file:design.md#viewer",
            "alice",
            "folder:root#viewer",
            Read::Contains,
            true,
        ),
        // The second parent holds after the first fails.
        (
            "file:twin.md#viewer",
            "alice",
            "folder:specs#parent",
            Read::Subjects,
            true,
        ),
        // An intersection's later operand is false: erin is banned.
        (
            "file:design.md#auditor",
            "erin",
            "file:design.md#auditor",
            Read::Contains,
            false,
        ),
        // The base fails, and bob is banned.
        (
            "file:design.md#viewer",
            "bob",
            "group:eng#member",
            Read::Contains,
            false,
        ),
    ];
    for (checked, who, failed, read, expected) in cases {
        let result = check_failing(&world, checked, who, failed, read).await;
        assert_eq!(
            result.ok(),
            Some(expected),
            "{checked} for {who} failing on {failed}"
        );
    }
}

#[tokio::test]
async fn a_store_error_anywhere_in_the_tree_fails_expand() {
    let mut world = world();
    // The docs theories exclude only computed subjectsets, which expand
    // leaves as references without a read.
    let unowned = Exclusion(Box::new(Computed("owner")), Box::new(This));
    fixture::declare(
        &mut world.theories,
        &world.dictionary,
        "file",
        "unowned",
        unowned,
    )
    .unwrap();
    let cases = [
        (
            "file:design.md#viewer",
            "file:design.md#viewer",
            Read::Subjects,
        ),
        (
            "file:design.md#viewer",
            "file:design.md#parent",
            Read::Subjects,
        ),
        (
            "file:design.md#unowned",
            "file:design.md#unowned",
            Read::Subjects,
        ),
        (
            "file:design.md#auditor",
            "file:design.md#auditor",
            Read::Subjects,
        ),
        (
            "file:design.md#viewer",
            "file:design.md#viewer",
            Read::Rewrite,
        ),
    ];
    for (checked, failed, read) in cases {
        let mut store = None;
        let extent = failing(&world, &mut store, checked, failed, read).await;
        assert!(
            matches!(extent.expand().await, Err(ExtentError::Store(_))),
            "{checked} failing on {failed}"
        );
    }
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
        &world.dictionary,
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
        &world.dictionary,
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
    let result = fixture::load_theories(json, &world.dictionary, &mut world.theories);
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
    let world = world();
    let before = head(&world).await;
    let fact = r#"[{ "set": "file:design.md#viewer", "identity": "zed" }]"#;
    fixture::load_facts(fact, &world.dictionary, &world.facts).unwrap();
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

const FILES: [&str; 2] = ["f0", "f1"];
const FOLDERS: [&str; 2] = ["d0", "d1"];
const GROUPS: [&str; 2] = ["g0", "g1"];
const PEOPLE: [&str; 3] = ["alice", "bob", "carol"];

/// Every subjectset of the random resources.
fn random_sets() -> Vec<String> {
    let mut sets = Vec::new();
    for file in FILES {
        for relation in ["owner", "parent", "editor", "viewer", "auditor", "banned"] {
            sets.push(format!("file:{file}#{relation}"));
        }
    }
    for folder in FOLDERS {
        for relation in ["owner", "parent", "viewer", "banned"] {
            sets.push(format!("folder:{folder}#{relation}"));
        }
    }
    for group in GROUPS {
        sets.push(format!("group:{group}#member"));
    }
    sets
}

/// Every fact a random world may hold, as fixture JSON.
fn candidate_facts() -> Vec<String> {
    let people = PEOPLE.map(|who| format!(r#""identity": "{who}""#));
    let groups = GROUPS.map(|group| format!(r#""subjectset": "group:{group}#member""#));
    let folders = FOLDERS.map(|folder| format!(r#""resource": "folder:{folder}""#));
    let mut facts = Vec::new();
    let mut add = |set: String, subjects: &[String]| {
        for subject in subjects {
            facts.push(format!(r#"{{ "set": "{set}", {subject} }}"#));
        }
    };
    for file in FILES {
        for relation in ["owner", "viewer", "auditor", "banned"] {
            add(format!("file:{file}#{relation}"), &people);
        }
        add(format!("file:{file}#viewer"), &groups);
        add(format!("file:{file}#parent"), &folders);
    }
    for folder in FOLDERS {
        for relation in ["viewer", "banned"] {
            add(format!("folder:{folder}#{relation}"), &people);
            add(format!("folder:{folder}#{relation}"), &groups);
        }
        add(format!("folder:{folder}#parent"), &folders);
    }
    for group in GROUPS {
        add(format!("group:{group}#member"), &people);
        add(format!("group:{group}#member"), &groups);
    }
    facts
}

/// Every read of a random subjectset that a probe may fail.
fn candidate_failures() -> Vec<(String, Read)> {
    let reads = [
        Read::Contains,
        Read::Subjectsets,
        Read::Subjects,
        Read::Rewrite,
    ];
    random_sets()
        .into_iter()
        .flat_map(|set| reads.map(|read| (set.clone(), read)))
        .collect()
}

/// The docs world plus `facts`, with every random name interned.
async fn random_world(facts: &[String]) -> World {
    let world = world();
    for (theory, ids) in [("file", FILES), ("folder", FOLDERS), ("group", GROUPS)] {
        let theory = world.dictionary.lookup(Pool::Theories, theory).await;
        let theory = TheoryId(theory.unwrap().unwrap());
        for id in ids {
            world.dictionary.intern(Pool::Resources(theory), id);
        }
    }
    for who in PEOPLE {
        world.dictionary.intern(Pool::Identities, who);
    }
    let facts = format!("[{}]", facts.join(","));
    fixture::load_facts(&facts, &world.dictionary, &world.facts).unwrap();
    world
}

/// The verdict, `None` when the check fails.
async fn verdict(probe: &Probe<'_>, set: Subjectset, who: Subject) -> Option<bool> {
    let revision = head(probe.world).await;
    agree(&Extent::new(probe, probe, set, revision), who)
        .await
        .ok()
}

async fn errors_follow_kleene_logic(
    facts: Vec<String>,
    failures: Vec<(String, Read)>,
    seed: u64,
) -> Result<(), TestCaseError> {
    let world = random_world(&facts).await;
    let mut failed = Vec::new();
    for (set, read) in failures {
        failed.push((subjectset(&world, &set).await.unwrap(), read));
    }
    let plain = Probe::new(&world);
    let shuffled = Probe {
        shuffle: Some(seed),
        ..Probe::new(&world)
    };
    let failing = Probe {
        failures: failed.clone(),
        ..Probe::new(&world)
    };
    let both = Probe {
        failures: failed,
        shuffle: Some(seed),
        ..Probe::new(&world)
    };
    for name in random_sets() {
        let set = subjectset(&world, &name).await.unwrap();
        for who in PEOPLE {
            let subject = identity(&world, who).await.unwrap();
            let expected = verdict(&plain, set, subject).await;
            prop_assert!(expected.is_some(), "{name} for {who}");
            let reordered = verdict(&shuffled, set, subject).await;
            prop_assert_eq!(reordered, expected, "{} for {} shuffled", name, who);
            // An error makes a verdict unknown, never wrong.
            let failed = verdict(&failing, set, subject).await;
            if failed.is_some() {
                prop_assert_eq!(failed, expected, "{} for {} failing", name, who);
            }
            let both = verdict(&both, set, subject).await;
            prop_assert_eq!(both, failed, "{} for {} failing, shuffled", name, who);
        }
    }
    Ok(())
}

proptest! {
    /// With any facts, any failed reads and any operand order, a check
    /// either gives the verdict it gives with nothing failing or fails, and
    /// which one doesn't depend on the order.
    #[test]
    fn errors_follow_kleene_logic_in_any_order(
        facts in subsequence(candidate_facts(), 0..=20),
        failures in subsequence(candidate_failures(), 0..=4),
        seed in any::<u64>(),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(errors_follow_kleene_logic(facts, failures, seed))?;
    }
}
