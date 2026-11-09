//! Namespaces are checked whole when declared.

use sungma::{
    id::{NamespaceId, RelationId},
    namespace::{
        Constraint, Declared, MAX_CONSTRAINTS, MAX_CYCLE_NAMES, MAX_REWRITE_DEPTH, Namespace,
        NamespaceError, Problem,
    },
    nonempty,
    rewrite::Rewrite::{self, Computed, Exclusion, This, Union},
};
use sungma_lang::namespace::{ErrorKind, Refusal, parse};
use sungma_names::resolve;
use sungma_names::store::Pool;
use sungma_node::testkit::World;

/// Parses and checks the document.
fn load(json: &str) -> Result<(), Refusal> {
    parse(json).map(|_| ()).map_err(Refusal)
}

/// The one namespace error a document is refused with.
fn refused(json: &str) -> NamespaceError {
    match parse(json) {
        Err(mut errors) if errors.len() == 1 => match errors.remove(0).kind {
            ErrorKind::Namespace(error) => error,
            other => panic!("expected a namespace error, got {other:?}"),
        },
        other => panic!("expected one error, got {other:?}"),
    }
}

/// Relation `relation` reading `this`, taking identities.
fn stored<R>(relation: R) -> Declared<R, &'static str> {
    (relation, This, vec![Constraint::Identity])
}

/// Relation `relation`, computed by `rewrite`.
fn computed<R>(relation: R, rewrite: Rewrite<R>) -> Declared<R, &'static str> {
    (relation, rewrite, vec![])
}

fn names(cycle: &[&str]) -> NamespaceError {
    NamespaceError::RewriteCycle {
        path: cycle.iter().map(|name| (*name).to_owned()).collect(),
        omitted: 0,
    }
}

#[tokio::test]
async fn a_refused_document_stores_no_namespace() {
    let world = World::new();
    let documents = [
        r#"{ "file": { "owner": { "constraints": ["identity"] } } }"#,
        r#"{ "folder": { "viewer": "editor" } }"#,
    ];
    let refused = world.load_namespaces(&documents).await;
    assert!(
        matches!(refused, Err(sungma_node::SeedError::Refused(_))),
        "{refused:?}"
    );
    let file = resolve::namespace(world.names(), &"file".parse().unwrap()).await;
    assert!(resolve::known(file).unwrap().is_none());
    let file = NamespaceId(world.intern(Pool::Namespaces, "file").await);
    assert!(world.latest(file).await.is_none());
}

#[test]
fn a_cycle_through_a_fact_is_left_to_evaluation() {
    let json = r#"{ "folder": {
        "parent": { "constraints": ["identity"] },
        "viewer": { "rewrite": "this | (parent, viewer)", "constraints": ["identity"] }
    } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_relation_declared_twice_is_refused() {
    let error = refused(
        r#"{ "file": { "owner": { "constraints": ["identity"] }, "owner": { "constraints": ["identity"] } } }"#,
    );
    assert_eq!(error, NamespaceError::DuplicateRelation("owner".to_owned()));
    assert_eq!(
        error.to_string(),
        "relation 'owner' is declared more than once"
    );
}

#[test]
fn a_namespace_document_is_refused_as_a_whole() {
    let result =
        load(r#"{ "file": { "owner": { "constraints": ["identity"] }, "viewer": "editor" } }"#);
    let Err(error) = result else {
        panic!("expected a refused document, got {result:?}");
    };
    assert_eq!(
        error.to_string(),
        "relation 'viewer': relation 'viewer' references 'editor', which the namespace doesn't declare"
    );
}

#[test]
fn a_computed_subjectset_must_be_declared() {
    let error = refused(r#"{ "file": { "viewer": "owner" } }"#);
    assert_eq!(
        error.to_string(),
        "relation 'viewer' references 'owner', which the namespace doesn't declare"
    );
}

#[test]
fn a_factset_must_be_declared() {
    let json = r#"{ "file": {
        "viewer": "(parent, viewer)"
    } }"#;
    assert_eq!(
        refused(json),
        NamespaceError::DanglingReference {
            relation: "viewer".to_owned(),
            target: "parent".to_owned(),
        }
    );
}

#[test]
fn a_relation_followed_in_another_namespace_is_left_to_the_write() {
    let json = r#"{ "file": {
        "parent": { "constraints": ["folder", "identity"] },
        "viewer": "(parent, anything)"
    } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_cycle_of_computed_subjectsets_is_refused() {
    let json = r#"{ "file": {
        "editor": "viewer",
        "viewer": { "rewrite": "this | editor", "constraints": ["identity"] }
    } }"#;
    let error = refused(json);
    assert_eq!(error, names(&["editor", "viewer", "editor"]));
    assert_eq!(
        error.to_string(),
        "rewrite cycle: editor -> viewer -> editor"
    );
}

#[test]
fn a_relation_excluding_itself_is_refused() {
    let json =
        r#"{ "file": { "viewer": { "rewrite": "this ! viewer", "constraints": ["identity"] } } }"#;
    assert_eq!(refused(json), names(&["viewer", "viewer"]));
}

#[test]
fn a_cycle_is_reported_from_where_it_closes() {
    let json = r#"{ "file": {
        "a": "b",
        "b": "c",
        "c": "b"
    } }"#;
    assert_eq!(refused(json), names(&["b", "c", "b"]));
}

#[test]
fn relations_sharing_a_target_are_no_cycle() {
    let json = r#"{ "file": {
        "a": "c",
        "b": "c | a",
        "c": { "constraints": ["identity"] }
    } }"#;
    assert!(load(json).is_ok());
}

/// A chain of single-operand unions, `levels` deep counting the leaf.
fn nested(levels: usize) -> Rewrite<&'static str> {
    nested_in(levels, |rewrite| Union(nonempty![rewrite]))
}

/// A chain of `wrap`s around `this`, `levels` deep counting the leaf.
fn nested_in(
    levels: usize,
    wrap: fn(Rewrite<&'static str>) -> Rewrite<&'static str>,
) -> Rewrite<&'static str> {
    (1..levels).fold(This, |rewrite, _| wrap(rewrite))
}

/// Checks a namespace declaring `deep` as `rewrite`, with its first
/// problem.
async fn declare(rewrite: Rewrite<&'static str>) -> Result<(), NamespaceError> {
    Namespace::<_, &str>::new(vec![("deep", rewrite, vec![Constraint::Identity])])
        .map(|_| ())
        .map_err(|mut problems| problems.remove(0).error)
}

#[tokio::test]
async fn a_rewrite_tree_may_be_max_depth_deep() {
    assert_eq!(declare(nested(MAX_REWRITE_DEPTH)).await, Ok(()));
}

#[tokio::test]
async fn a_deeper_rewrite_tree_is_refused() {
    let error = declare(nested(MAX_REWRITE_DEPTH + 1)).await.unwrap_err();
    assert_eq!(error, NamespaceError::TooDeep);
    assert_eq!(
        error.to_string(),
        "a rewrite tree deeper than 100 levels is refused"
    );
}

#[tokio::test]
async fn a_deeper_exclusion_base_is_refused() {
    let base = nested_in(MAX_REWRITE_DEPTH + 1, |rewrite| {
        Exclusion(Box::new(rewrite), Box::new(This))
    });
    assert_eq!(declare(base).await, Err(NamespaceError::TooDeep));
}

#[tokio::test]
async fn a_deeper_excluded_operand_is_refused() {
    let excluded = nested_in(MAX_REWRITE_DEPTH + 1, |rewrite| {
        Exclusion(Box::new(This), Box::new(rewrite))
    });
    assert_eq!(declare(excluded).await, Err(NamespaceError::TooDeep));
}

#[tokio::test]
async fn redeclaring_a_relation_checks_the_whole_namespace_by_id() {
    let world = World::new();
    world
        .declare("file", "owner", This, &["identity"])
        .await
        .unwrap();
    world
        .declare("file", "editor", Computed("owner"), &[])
        .await
        .unwrap();
    // Relations have their own pool, so owner is 0 and editor is 1. The
    // kept editor is checked before the redeclared owner, which now closes
    // a cycle through it.
    let error = world
        .declare("file", "owner", Computed("editor"), &[])
        .await
        .unwrap_err();
    assert_eq!(error, names(&["1", "0", "1"]));
}

#[test]
fn a_namespace_built_with_a_relation_twice_is_refused() {
    let result = Namespace::new(vec![stored(RelationId(0)), stored(RelationId(0))]);
    assert_eq!(
        result.err(),
        Some(vec![Problem {
            at: 1,
            error: NamespaceError::DuplicateRelation("0".to_owned()),
        }])
    );
}

/// A namespace built in code bypasses the document limits, so its checks must
/// hold up at any size: a chain far longer than a document allows.
#[test]
fn a_long_chain_built_in_code_is_checked_without_a_crash() {
    let count = 100_000;
    let chain: Vec<_> = (0..count)
        .map(|i| {
            if i + 1 == count {
                stored(i)
            } else {
                computed(i, Computed(i + 1))
            }
        })
        .collect();
    assert!(Namespace::new(chain).is_ok());
}

#[test]
fn a_large_cycle_built_in_code_is_one_problem() {
    let count = 100_000;
    let ring: Vec<_> = (0..count)
        .map(|i| computed(i, Computed((i + 1) % count)))
        .collect();
    let problems = Namespace::new(ring).unwrap_err();
    let [
        Problem {
            at: 0,
            error: NamespaceError::RewriteCycle { path, omitted },
        },
    ] = problems.as_slice()
    else {
        panic!("expected one cycle, got {} problems", problems.len());
    };
    assert_eq!(path.len(), MAX_CYCLE_NAMES + 1);
    assert_eq!(*omitted, count - MAX_CYCLE_NAMES);
    assert_eq!(path.first(), path.last());
}

#[test]
fn every_problem_is_returned_in_declaration_order() {
    let problems = Namespace::new(vec![
        computed(0, Computed(9)),
        computed(1, Computed(8)),
        computed(2, Computed(3)),
        computed(3, Computed(2)),
        stored(0),
    ])
    .unwrap_err();
    let found: Vec<_> = problems.iter().map(|problem| problem.at).collect();
    assert_eq!(found, [0, 1, 2, 4]);
}

#[test]
fn a_cycle_error_names_a_bounded_path() {
    let error = NamespaceError::RewriteCycle {
        path: vec!["a".to_owned(), "b".to_owned(), "a".to_owned()],
        omitted: 3,
    };
    assert_eq!(
        error.to_string(),
        "rewrite cycle: a -> b -> ... 3 more -> a"
    );
}

#[test]
fn a_namespace_keeps_its_declaration_order() {
    let namespace = Namespace::new(vec![stored(2), computed(0, Computed(2)), stored(1)]).unwrap();
    let order: Vec<_> = namespace
        .relations()
        .map(|(relation, _)| *relation)
        .collect();
    assert_eq!(order, [2, 0, 1]);
}

#[test]
fn a_cycle_path_stays_within_its_relations() {
    // a and b reach each other; b also leads out to c, which leads nowhere back.
    let problems = Namespace::new(vec![
        computed(0, Computed(1)),
        computed(1, Union(nonempty![Computed(2), Computed(0)])),
        stored(2),
    ])
    .unwrap_err();
    let [Problem { at: 0, error }] = problems.as_slice() else {
        panic!("expected one cycle, got {problems:?}");
    };
    assert_eq!(*error, names(&["0", "1", "0"]));
}

// Constraints: a relation that reads `this` declares the kinds of subject
// its facts may bind, and only such a relation declares them.

#[test]
fn a_relation_reading_this_without_constraints_is_refused() {
    let errors = parse(r#"{ "file": { "owner": "this", "viewer": "owner | this" } }"#).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert!(matches!(
        &errors[0].kind,
        ErrorKind::Namespace(NamespaceError::Unconstrained(owner)) if owner == "owner"
    ));
    assert_eq!(
        errors[1].to_string(),
        "relation 'viewer': relation 'viewer' reads 'this' and declares no constraints"
    );
}

#[test]
fn a_computed_relation_with_constraints_is_refused() {
    let json = r#"{ "file": {
        "owner": { "constraints": ["identity"] },
        "viewer": { "rewrite": "owner", "constraints": ["identity"] }
    } }"#;
    let error = refused(json);
    assert_eq!(
        error,
        NamespaceError::ConstrainedComputed("viewer".to_owned())
    );
    assert_eq!(
        error.to_string(),
        "relation 'viewer' declares constraints and doesn't read 'this'"
    );
}

#[test]
fn a_rewrite_left_out_reads_this() {
    let document = parse(r#"{ "file": { "owner": { "constraints": ["identity"] } } }"#).unwrap();
    let owner = "owner".parse().unwrap();
    assert_eq!(
        document.relations.rewrite(&owner).map(|rewrite| &**rewrite),
        Some(&This)
    );
    assert_eq!(
        document.relations.constraints(&owner),
        [Constraint::Identity]
    );
}

#[test]
fn this_anywhere_in_a_rewrite_needs_constraints() {
    for rewrite in ["owner ! this", "owner & (this | owner)", "(this)"] {
        let json = format!(
            r#"{{ "file": {{ "owner": {{ "constraints": ["identity"] }}, "r": "{rewrite}" }} }}"#
        );
        assert_eq!(
            refused(&json),
            NamespaceError::Unconstrained("r".to_owned()),
            "{rewrite}"
        );
    }
}

#[test]
fn a_constraint_written_twice_is_refused_once() {
    let json = r#"{ "file": {
        "owner": { "constraints": ["identity", "folder", "identity", "identity", "folder"] }
    } }"#;
    let errors = parse(json).unwrap_err();
    let found: Vec<_> = errors.iter().map(|error| error.kind.to_string()).collect();
    assert_eq!(
        found,
        [
            "relation 'owner' declares constraint 'identity' more than once",
            "relation 'owner' declares constraint 'folder' more than once",
        ]
    );
}

/// A namespace whose `owner` takes `count` distinct namespaces.
fn constrained(count: usize) -> String {
    let constraints: Vec<_> = (0..count).map(|i| format!(r#""n{i}""#)).collect();
    format!(
        r#"{{ "file": {{ "owner": {{ "constraints": [{}] }} }} }}"#,
        constraints.join(", ")
    )
}

#[test]
fn a_relation_declares_at_most_max_constraints() {
    assert!(load(&constrained(MAX_CONSTRAINTS)).is_ok());
    let error = refused(&constrained(MAX_CONSTRAINTS + 1));
    assert_eq!(
        error,
        NamespaceError::TooManyConstraints {
            relation: "owner".to_owned(),
            count: MAX_CONSTRAINTS + 1,
        }
    );
    assert_eq!(
        error.to_string(),
        "relation 'owner' declares 33 constraints, more than 32"
    );
}

#[test]
fn a_constraint_naming_its_own_namespace_needs_the_relation_declared() {
    let json = r#"{ "group": {
        "member": { "constraints": ["group#member", "group#admin", "identity"] }
    } }"#;
    let error = refused(json);
    assert_eq!(
        error,
        NamespaceError::UndeclaredConstraint {
            relation: "member".to_owned(),
            constraint: "group#admin".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "relation 'member' has constraint 'group#admin', whose namespace doesn't declare its relation"
    );
}

#[test]
fn a_constraint_naming_another_namespace_is_left_to_the_write() {
    let json = r#"{ "file": { "owner": { "constraints": ["group#member", "folder"] } } }"#;
    assert!(load(json).is_ok());
}

#[test]
fn a_followed_relation_its_own_namespace_lacks_is_refused() {
    let json = r#"{ "folder": {
        "parent": { "constraints": ["folder", "identity", "folder#parent"] },
        "viewer": "(parent, viewer) | (parent, missing) | (parent, missing)"
    } }"#;
    let error = refused(json);
    assert_eq!(
        error,
        NamespaceError::UndeclaredFollowed {
            relation: "viewer".to_owned(),
            namespace: "folder".to_owned(),
            followed: "missing".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "relation 'viewer' follows 'missing', which namespace 'folder' doesn't declare"
    );
}

#[test]
fn a_namespace_named_identity_is_refused() {
    let errors = parse(r#"{ "identity": {} }"#).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].to_string(), "'identity' is reserved");
    assert!(load(r#"{ "identities": {} }"#).is_ok());
    assert!(load(r#"{ "drive.identity": {} }"#).is_ok());
}

/// The checks that reach past a namespace answer for its own references,
/// and leave the rest to the caller's answer.
#[test]
fn references_ask_the_caller_about_other_namespaces() {
    let namespace = Namespace::new(vec![
        (
            "parent",
            This,
            vec![Constraint::Resource("folder"), Constraint::Resource("file")],
        ),
        (
            "member",
            This,
            vec![
                Constraint::Subjectset("group", "member"),
                Constraint::Identity,
            ],
        ),
        (
            "viewer",
            Rewrite::FactTo {
                factset: "parent",
                computed: "member",
            },
            vec![],
        ),
    ])
    .unwrap();
    let known = |namespace: &&str, relation: &&str| match *namespace {
        "group" => Some(*relation == "owner"),
        _ => None,
    };
    let problems = namespace.references(&"file", known);
    let errors: Vec<_> = problems
        .iter()
        .map(|problem| (problem.at, &problem.error))
        .collect();
    assert_eq!(
        errors,
        [(
            1,
            &NamespaceError::UndeclaredConstraint {
                relation: "member".to_owned(),
                constraint: "group#member".to_owned(),
            }
        ),]
    );
    let none = |_: &&str, _: &&str| Some(false);
    let problems = namespace.references(&"file", none);
    let errors: Vec<_> = problems
        .iter()
        .map(|problem| problem.error.to_string())
        .collect();
    assert_eq!(
        errors,
        [
            "relation 'member' has constraint 'group#member', whose namespace doesn't declare its relation",
            "relation 'viewer' follows 'member', which namespace 'folder' doesn't declare",
        ]
    );
    let all = |_: &&str, _: &&str| Some(true);
    assert!(namespace.references(&"file", all).is_empty());
}

#[test]
fn a_namespace_lists_the_namespaces_its_constraints_name_once() {
    let namespace = Namespace::new(vec![
        (
            "a",
            This,
            vec![
                Constraint::Resource("folder"),
                Constraint::Identity,
                Constraint::Subjectset("group", "member"),
            ],
        ),
        ("b", This, vec![Constraint::Subjectset("folder", "owner")]),
    ])
    .unwrap();
    assert_eq!(namespace.namespaces(), [&"folder", &"group"]);
    assert_eq!(namespace.names(), [&"a", &"member", &"b", &"owner"]);
}

/// The namespaces references judge: a constraint's `namespace#relation`,
/// and each a followed factset takes, but not one only a plain resource
/// constraint names.
#[test]
fn a_namespace_lists_the_namespaces_its_references_judge_once() {
    let namespace = Namespace::new(vec![
        (
            "parent",
            This,
            vec![Constraint::Resource("folder"), Constraint::Identity],
        ),
        ("owner", This, vec![Constraint::Resource("drive")]),
        (
            "member",
            This,
            vec![Constraint::Subjectset("group", "member")],
        ),
        (
            "viewer",
            Rewrite::Union(nonempty![
                Rewrite::FactTo {
                    factset: "parent",
                    computed: "viewer",
                },
                Rewrite::FactTo {
                    factset: "member",
                    computed: "viewer",
                },
            ]),
            vec![],
        ),
    ])
    .unwrap();
    assert_eq!(namespace.judged(), [&"group", &"folder"]);
    let plain = Namespace::new(vec![("owner", This, vec![Constraint::Resource("drive")])]).unwrap();
    assert!(plain.judged().is_empty());
}

#[test]
fn a_constraint_admits_only_its_kind_of_subject() {
    use sungma::graph::{Resource, Subject, Subjectset};
    let resource = |namespace| Resource {
        namespace: NamespaceId(namespace),
        id: "x".parse().unwrap(),
    };
    let identity = Subject::Identity("alice".parse().unwrap());
    let member = Subject::ResourceMember(resource(1));
    let set = |namespace, relation| {
        Subject::Subjectset(Subjectset {
            resource: resource(namespace),
            relation: RelationId(relation),
        })
    };
    let subjects = [
        identity,
        member,
        set(1, 2),
        set(1, 3),
        set(2, 2),
        Subject::ResourceMember(resource(2)),
    ];
    for (constraint, admitted) in [
        (
            Constraint::Identity,
            [true, false, false, false, false, false],
        ),
        (
            Constraint::Resource(NamespaceId(1)),
            [false, true, false, false, false, false],
        ),
        (
            Constraint::Subjectset(NamespaceId(1), RelationId(2)),
            [false, false, true, false, false, false],
        ),
    ] {
        let found: Vec<_> = subjects
            .iter()
            .map(|subject| constraint.admits(subject))
            .collect();
        assert_eq!(found, admitted, "{constraint}");
    }
}

/// A namespace built in code bypasses the document limits, so a relation
/// with far more constraints than the limit is refused without a crash.
#[test]
fn far_more_constraints_built_in_code_are_one_problem() {
    let constraints: Vec<_> = (0..100_000).map(Constraint::Resource).collect();
    let problems = Namespace::new(vec![(0, This, constraints)]).unwrap_err();
    assert_eq!(
        problems,
        [Problem {
            at: 0,
            error: NamespaceError::TooManyConstraints {
                relation: "0".to_owned(),
                count: 100_000,
            },
        }]
    );
}
