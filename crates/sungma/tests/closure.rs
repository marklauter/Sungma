//! Contains and Expand against the namespaces in [`common`].

mod common;

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use common::{World, declare, head, identity, subjectset, world};
use proptest::{prelude::*, sample::subsequence, test_runner::TestCaseError};
use sungma::{
    graph::{Subject, Subjectset},
    id::NamespaceId,
    namespace::Namespace,
    non_empty::NonEmpty,
    revision::Revision,
    rewrite::Rewrite::{self, Computed, Exclusion, Intersection, This, Union},
    store::{Pool, StoreError},
    version::{History, Version},
};
use sungma_check::{
    closure::{Closure, ClosureError, Expansion, MAX_DEPTH},
    store::FactReader,
    store::NamespaceReader,
};
use sungma_lang::fact::load_facts;

/// Whether an identity is in the closure of `namespace:id#relation` at `revision`.
async fn check_at(
    world: &World,
    set: &str,
    who: &str,
    revision: Revision,
) -> Result<bool, ClosureError> {
    let Some(set) = subjectset(world, set).await else {
        return Ok(false);
    };
    let subject = identity(who);
    agree(
        &Closure::new(&world.store, &world.store, &set, revision),
        subject,
    )
    .await
}

/// [`Closure::contains`], checked against [`Closure::decide`]: the same
/// verdict, or both fail.
async fn agree<T: NamespaceReader + Sync, F: FactReader + Sync>(
    closure: &Closure<'_, T, F>,
    subject: Subject,
) -> Result<bool, ClosureError> {
    let fast = closure.contains(&subject).await;
    let decided = closure.decide(subject).await;
    match (&fast, &decided) {
        (Ok(fast), Ok(decided)) => assert_eq!(*fast, decided.outcome.is_allowed()),
        (Err(_), Err(_)) => {}
        _ => panic!("contains {fast:?} and decide {decided:?} disagree"),
    }
    fast
}

/// [`check_at`] the latest revision.
async fn check(world: &World, set: &str, who: &str) -> Result<bool, ClosureError> {
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
        { "namespace": "group", "resource": "ring_a", "relation": "member", "subject": { "namespace": "group", "resource": "ring_b", "relation": "member" } },
        { "namespace": "group", "resource": "ring_b", "relation": "member", "subject": { "namespace": "group", "resource": "ring_a", "relation": "member" } },
        { "namespace": "group", "resource": "cyc_a", "relation": "member", "subject": { "namespace": "group", "resource": "cyc_b", "relation": "member" } },
        { "namespace": "group", "resource": "cyc_b", "relation": "member", "subject": { "namespace": "group", "resource": "cyc_a", "relation": "member" } },
        { "namespace": "group", "resource": "cyc_a", "relation": "member", "subject": { "namespace": "group", "resource": "cyc_c", "relation": "member" } },
        { "namespace": "group", "resource": "cyc_c", "relation": "member", "subject": { "identity": "alice" } },
        { "namespace": "folder", "resource": "open", "relation": "viewer", "subject": { "identity": "alice" } },
        { "namespace": "folder", "resource": "open", "relation": "banned", "subject": { "namespace": "group", "resource": "ring_a", "relation": "member" } },
        { "namespace": "file", "resource": "ring.md", "relation": "viewer", "subject": { "namespace": "group", "resource": "ring_a", "relation": "member" } },
        { "namespace": "file", "resource": "ring.md", "relation": "owner", "subject": { "identity": "alice" } }
    ]"#;
    load_facts(facts, &world.dictionary, &world.store).unwrap();
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

/// Checks `group:g0#member` down a chain of `length` subjectsets, with
/// `more` facts loaded after the chain.
async fn chain(length: usize, more: &[&str]) -> Result<bool, ClosureError> {
    let world = world();
    let mut facts: Vec<String> = (1..length)
        .map(|i| {
            format!(
                r#"{{ "namespace": "group", "resource": "g{}", "relation": "member", "subject": {{ "namespace": "group", "resource": "g{i}", "relation": "member" }} }}"#,
                i - 1
            )
        })
        .collect();
    facts.extend(more.iter().map(ToString::to_string));
    let facts = format!("[{}]", facts.join(","));
    load_facts(&facts, &world.dictionary, &world.store).unwrap();
    check(&world, "group:g0#member", "alice").await
}

#[tokio::test]
async fn a_chain_may_be_max_depth_long() {
    assert!(!chain(MAX_DEPTH, &[]).await.unwrap());
}

#[tokio::test]
async fn a_longer_chain_exceeds_depth() {
    let result = chain(MAX_DEPTH + 1, &[]).await;
    assert!(matches!(
        result,
        Err(ClosureError::DepthExceeded(MAX_DEPTH))
    ));
}

#[tokio::test]
async fn passing_the_depth_limit_ends_the_check() {
    // g1 is interned before `holds`, so the chain is walked first, and
    // `holds` would find alice after it.
    let holds = [
        r#"{ "namespace": "group", "resource": "g0", "relation": "member", "subject": { "namespace": "group", "resource": "holds", "relation": "member" } }"#,
        r#"{ "namespace": "group", "resource": "holds", "relation": "member", "subject": { "identity": "alice" } }"#,
    ];
    let result = chain(MAX_DEPTH + 1, &holds).await;
    assert!(matches!(
        result,
        Err(ClosureError::DepthExceeded(MAX_DEPTH))
    ));
    assert!(chain(MAX_DEPTH, &holds).await.unwrap());
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
async fn a_parent_of_an_undeclared_namespace_is_empty() {
    let world = world();
    // `ghost` is a known name that no version declares, as for a retired
    // namespace.
    world.dictionary.intern(Pool::Namespaces, "ghost").unwrap();
    let fact = r#"[{ "namespace": "file", "resource": "haunted.md", "relation": "parent", "subject": { "namespace": "ghost", "resource": "attic" } }]"#;
    load_facts(fact, &world.dictionary, &world.store).unwrap();
    assert!(
        !check(&world, "file:haunted.md#viewer", "alice")
            .await
            .unwrap()
    );
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Read {
    Written,
    Subjectsets,
    Subjects,
    /// The version of the subjectset's namespace, for any resource and
    /// relation.
    Version,
}

/// The docs namespace and fact stores, failing the listed reads. With a seed,
/// the operands of every union and intersection and the subjects of every
/// read come back shuffled.
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

    /// Shuffles the subjects read under `set`, when the probe shuffles.
    fn permute<T>(&self, subjects: &mut [T], set: &Subjectset) {
        if let Some(seed) = self.shuffle {
            permute(subjects, &mut seed_for(seed, set));
        }
    }

    fn fail(&self, set: &Subjectset, read: Read) -> Result<(), StoreError> {
        if self
            .failures
            .iter()
            .any(|(failed, at)| failed == set && *at == read)
        {
            return Err(StoreError("injected".to_owned()));
        }
        Ok(())
    }
}

/// A seed for one read, mixed from the probe's seed and what is read, so
/// each read is permuted its own way.
fn seed_for(seed: u64, read: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    (seed, read).hash(&mut hasher);
    hasher.finish()
}

/// Shuffles `items` in place with a generator seeded by `seed`.
fn permute<T>(items: &mut [T], seed: &mut u64) {
    for i in (1..items.len()).rev() {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        items.swap(i, (*seed >> 33) as usize % (i + 1));
    }
}

/// `rewrite` with the operands of each union and intersection permuted by
/// `seed`. An exclusion's sides keep their places.
fn shuffled(rewrite: &Rewrite, seed: &mut u64) -> Rewrite {
    let mut operands = |operands: &[Rewrite]| {
        let mut operands: Vec<Rewrite> = operands.iter().map(|op| shuffled(op, seed)).collect();
        permute(&mut operands, seed);
        NonEmpty::new(operands).expect("a permutation keeps every operand")
    };
    match rewrite {
        Union(ops) => Union(operands(ops)),
        Intersection(ops) => Intersection(operands(ops)),
        Exclusion(base, excluded) => Exclusion(
            Box::new(shuffled(base, seed)),
            Box::new(shuffled(excluded, seed)),
        ),
        leaf => leaf.clone(),
    }
}

impl NamespaceReader for Probe<'_> {
    async fn version(
        &self,
        namespace: NamespaceId,
        revision: Revision,
    ) -> Result<Option<Arc<Version>>, StoreError> {
        let failed = self
            .failures
            .iter()
            .any(|(set, read)| *read == Read::Version && set.resource.namespace == namespace);
        if failed {
            return Err(StoreError("injected".to_owned()));
        }
        let version = self.world.store.version(namespace, revision).await?;
        Ok(match (self.shuffle, version) {
            (Some(seed), Some(version)) => Some(reordered(&version, namespace, seed)),
            (_, version) => version,
        })
    }
}

/// `version` with every rewrite [`shuffled`]. The copy is the first
/// version of a history of its own, at revision 0, so its epochs are 0 and
/// every fact under a relation it declares counts. That matches the
/// version only where every fact was written after its relation's epoch,
/// as in the worlds the shuffling probe reads.
fn reordered(version: &Version, namespace: NamespaceId, seed: u64) -> Arc<Version> {
    let relations = version
        .namespace()
        .relations()
        .map(|(relation, rewrite)| {
            let mut seed = seed_for(seed, (namespace, *relation));
            (*relation, shuffled(rewrite, &mut seed))
        })
        .collect();
    let mut history = History::default();
    history
        .write(Namespace::new(relations).unwrap(), Revision(0))
        .unwrap();
    Arc::clone(history.latest().unwrap())
}

impl FactReader for Probe<'_> {
    async fn head(&self) -> Result<Revision, StoreError> {
        self.world.store.head().await
    }

    async fn written(
        &self,
        set: &Subjectset,
        subject: &Subject,
        revision: Revision,
    ) -> Result<Option<Revision>, StoreError> {
        self.fail(set, Read::Written)?;
        self.world.store.written(set, subject, revision).await
    }

    async fn subjectsets(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subjectset, Revision)>, StoreError> {
        self.fail(set, Read::Subjectsets)?;
        let mut nested = self.world.store.subjectsets(set, revision).await?;
        self.permute(&mut nested, set);
        Ok(nested)
    }

    async fn subjects(
        &self,
        set: &Subjectset,
        revision: Revision,
    ) -> Result<Vec<(Subject, Revision)>, StoreError> {
        self.fail(set, Read::Subjects)?;
        let mut subjects = self.world.store.subjects(set, revision).await?;
        self.permute(&mut subjects, set);
        Ok(subjects)
    }
}

/// An [`Closure`] over `checked` whose stores fail `read` of `failed`.
async fn failing<'a>(
    world: &'a World,
    store: &'a mut Option<Probe<'a>>,
    checked: &'a Subjectset,
    failed: &str,
    read: Read,
) -> Closure<'a, Probe<'a>, Probe<'a>> {
    let set = subjectset(world, failed).await.unwrap();
    let store = store.insert(Probe {
        failures: vec![(set, read)],
        ..Probe::new(world)
    });
    Closure::new(store, store, checked, head(world).await)
}

/// Checks `who` against `checked` with `read` of `failed` failing.
async fn check_failing(
    world: &World,
    checked: &str,
    who: &str,
    failed: &str,
    read: Read,
) -> Result<bool, ClosureError> {
    let who = identity(who);
    let checked = subjectset(world, checked).await.unwrap();
    let mut store = None;
    let closure = failing(world, &mut store, &checked, failed, read).await;
    agree(&closure, who).await
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
            Read::Written,
        ),
        (
            "file:design.md#viewer",
            "alice",
            "folder:specs#viewer",
            Read::Version,
        ),
        // The base holds, and whether alice is banned is unknown.
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#banned",
            Read::Written,
        ),
        // alice is stored as an auditor, and whether she views is unknown.
        (
            "file:design.md#auditor",
            "alice",
            "group:eng#member",
            Read::Written,
        ),
    ];
    for (checked, who, failed, read) in cases {
        let result = check_failing(&world, checked, who, failed, read).await;
        assert!(
            matches!(result, Err(ClosureError::Store(_))),
            "{checked} for {who} failing on {failed}: {result:?}"
        );
    }
}

#[tokio::test]
async fn an_operand_that_settles_the_check_outweighs_a_store_error() {
    let world = world();
    let facts = r#"[
        { "namespace": "file", "resource": "twin.md", "relation": "parent", "subject": { "namespace": "folder", "resource": "specs" } },
        { "namespace": "file", "resource": "twin.md", "relation": "parent", "subject": { "namespace": "folder", "resource": "shared" } },
        { "namespace": "folder", "resource": "shared", "relation": "viewer", "subject": { "identity": "alice" } }
    ]"#;
    load_facts(facts, &world.dictionary, &world.store).unwrap();
    let cases = [
        // A union's later operand holds: alice views through the parent.
        (
            "file:design.md#viewer",
            "alice",
            "file:design.md#viewer",
            Read::Written,
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
            Read::Written,
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
            Read::Written,
            false,
        ),
        // The base fails, and bob is banned.
        (
            "file:design.md#viewer",
            "bob",
            "group:eng#member",
            Read::Written,
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
    let world = world();
    // The docs namespaces exclude only computed subjectsets, which expand
    // leaves as references without a read.
    let unowned = Exclusion(Box::new(Computed("owner")), Box::new(This));
    declare(&world.store, &world.dictionary, "file", "unowned", unowned).unwrap();
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
            Read::Version,
        ),
    ];
    for (checked, failed, read) in cases {
        let checked_set = subjectset(&world, checked).await.unwrap();
        let mut store = None;
        let closure = failing(&world, &mut store, &checked_set, failed, read).await;
        assert!(
            matches!(closure.expand().await, Err(ClosureError::Store(_))),
            "{checked} failing on {failed}"
        );
    }
}

#[tokio::test]
async fn expand_of_an_undeclared_relation_is_none() {
    let world = world();
    let viewer = subjectset(&world, "group:eng#viewer").await.unwrap();
    let expansion = Closure::new(&world.store, &world.store, &viewer, head(&world).await)
        .expand()
        .await
        .unwrap();
    assert_eq!(expansion, None);
}

#[tokio::test]
async fn expand_keeps_an_intersection() {
    let world = world();
    let auditor = subjectset(&world, "file:design.md#auditor").await.unwrap();
    let expansion = Closure::new(&world.store, &world.store, &auditor, head(&world).await)
        .expand()
        .await
        .unwrap();
    let alice = identity("alice");
    let dave = identity("dave");
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
    let fact = r#"[{ "namespace": "file", "resource": "design.md", "relation": "viewer", "subject": { "identity": "zed" } }]"#;
    load_facts(fact, &world.dictionary, &world.store).unwrap();
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
    let expansion = Closure::new(&world.store, &world.store, &viewer, head(&world).await)
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
    let expansion = Closure::new(&world.store, &world.store, &root_viewer, head(&world).await)
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
    let erin = identity("erin");
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

/// Every fact a random world may hold, as fact JSON.
fn candidate_facts() -> Vec<String> {
    let people = PEOPLE.map(|who| format!(r#"{{ "identity": "{who}" }}"#));
    let groups = GROUPS.map(|group| {
        format!(r#"{{ "namespace": "group", "resource": "{group}", "relation": "member" }}"#)
    });
    let folders =
        FOLDERS.map(|folder| format!(r#"{{ "namespace": "folder", "resource": "{folder}" }}"#));
    let mut facts = Vec::new();
    let mut add = |namespace: &str, resource: &str, relation: &str, subjects: &[String]| {
        for subject in subjects {
            facts.push(format!(
                r#"{{ "namespace": "{namespace}", "resource": "{resource}", "relation": "{relation}", "subject": {subject} }}"#
            ));
        }
    };
    for file in FILES {
        for relation in ["owner", "viewer", "auditor", "banned"] {
            add("file", file, relation, &people);
        }
        add("file", file, "viewer", &groups);
        add("file", file, "parent", &folders);
    }
    for folder in FOLDERS {
        for relation in ["viewer", "banned"] {
            add("folder", folder, relation, &people);
            add("folder", folder, relation, &groups);
        }
        add("folder", folder, "parent", &folders);
    }
    for group in GROUPS {
        add("group", group, "member", &people);
        add("group", group, "member", &groups);
    }
    facts
}

/// Every read of a random subjectset that a probe may fail.
fn candidate_failures() -> Vec<(String, Read)> {
    let reads = [
        Read::Written,
        Read::Subjectsets,
        Read::Subjects,
        Read::Version,
    ];
    random_sets()
        .into_iter()
        .flat_map(|set| reads.map(|read| (set.clone(), read)))
        .collect()
}

/// The docs world plus `facts`.
fn random_world(facts: &[String]) -> World {
    let world = world();
    let facts = format!("[{}]", facts.join(","));
    load_facts(&facts, &world.dictionary, &world.store).unwrap();
    world
}

/// The verdict, `None` when the check fails.
async fn verdict(probe: &Probe<'_>, set: &Subjectset, who: &Subject) -> Option<bool> {
    let revision = head(probe.world).await;
    agree(&Closure::new(probe, probe, set, revision), who.clone())
        .await
        .ok()
}

async fn errors_follow_kleene_logic(
    facts: Vec<String>,
    failures: Vec<(String, Read)>,
    seed: u64,
) -> Result<(), TestCaseError> {
    let world = random_world(&facts);
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
            let subject = identity(who);
            let expected = verdict(&plain, &set, &subject).await;
            prop_assert!(expected.is_some(), "{name} for {who}");
            let reordered = verdict(&shuffled, &set, &subject).await;
            prop_assert_eq!(reordered, expected, "{} for {} shuffled", name, who);
            // An error makes a verdict unknown, never wrong.
            let failed = verdict(&failing, &set, &subject).await;
            if failed.is_some() {
                prop_assert_eq!(failed, expected, "{} for {} failing", name, who);
            }
            let both = verdict(&both, &set, &subject).await;
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
