//! Decisions, replay, and the audited Check service, against the namespaces
//! in [`common`].

mod common;

use std::time::SystemTime;

use common::{World, declare, head, identity, subjectset, world};
use sungma::{
    closure::Closure,
    decision::{Decision, Outcome, SEMANTICS, Semantics},
    graph::{Fact, Subject},
    replay::{Replay, replay},
    revision::Revision,
    rewrite::Rewrite::This,
};
use sungma_check::{
    resolve, {CheckRequest, CheckService, MemoryAuditLog, RequestContext, SubjectName, Verdict},
};
use sungma_lang::fact::load_facts;

/// `namespace:id#relation@subject`
async fn fact(world: &World, set: &str, subject: Subject) -> Fact {
    Fact {
        subjectset: subjectset(world, set).await.unwrap(),
        subject,
    }
}

/// `namespace:id#...`
async fn member(world: &World, resource: &str) -> Subject {
    let resource = resolve::resource(&world.dictionary, &resource.parse().unwrap()).await;
    Subject::ResourceMember(resource.unwrap().unwrap())
}

async fn decide(world: &World, set: &str, who: &str, revision: Revision) -> Decision {
    let set = subjectset(world, set).await.unwrap();
    let subject = identity(who);
    Closure::new(&world.namespaces, &world.facts, set, revision)
        .decide(subject)
        .await
        .unwrap()
}

fn grounds(decision: &Decision) -> &[Fact] {
    match &decision.outcome {
        Outcome::Allowed { grounds } => grounds,
        Outcome::Denied => panic!("expected an allowed decision"),
    }
}

#[tokio::test]
async fn grounds_cite_the_facts_along_the_granting_path() {
    let world = world();
    let decision = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;

    let eng = subjectset(&world, "group:eng#member").await.unwrap();
    let alice = identity("alice");
    let expected = [
        fact(
            &world,
            "file:design.md#parent",
            member(&world, "folder:specs").await,
        )
        .await,
        fact(
            &world,
            "folder:specs#parent",
            member(&world, "folder:root").await,
        )
        .await,
        fact(&world, "folder:root#viewer", Subject::Subjectset(eng)).await,
        fact(&world, "group:eng#member", alice.clone()).await,
    ];
    assert_eq!(grounds(&decision), expected);
    assert_eq!(decision.subject, alice);
    assert_eq!(decision.semantics, SEMANTICS);
}

#[tokio::test]
async fn rewrite_steps_without_facts_leave_no_grounds() {
    let world = world();
    let decision = decide(&world, "file:design.md#viewer", "carol", head(&world).await).await;
    let carol = identity("carol");
    // viewer → editor → owner: only the owner fact is stored.
    assert_eq!(
        grounds(&decision),
        [fact(&world, "file:design.md#owner", carol).await]
    );
}

#[tokio::test]
async fn intersection_cites_every_operand() {
    let world = world();
    let decision = decide(
        &world,
        "file:design.md#auditor",
        "alice",
        head(&world).await,
    )
    .await;
    let alice = identity("alice");
    let cited = grounds(&decision);
    assert_eq!(
        cited[0],
        fact(&world, "file:design.md#auditor", alice).await
    );
    assert_eq!(
        cited.len(),
        5,
        "the auditor fact plus the four viewer grounds"
    );
}

#[tokio::test]
async fn denied_cites_nothing() {
    let world = world();
    let revision = head(&world).await;
    let decision = decide(&world, "file:design.md#viewer", "bob", revision).await;
    assert_eq!(decision.outcome, Outcome::Denied);
    assert_eq!(decision.revision, revision);
}

#[tokio::test]
async fn replay_at_the_recorded_revision_ignores_later_writes() {
    let world = world();
    let decision = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;

    let ban = r#"[{ "namespace": "file", "resource": "design.md", "relation": "banned", "subject": { "identity": "alice" } }]"#;
    load_facts(ban, &world.dictionary, &world.facts).unwrap();

    let replayed = replay(&decision, &world.namespaces, &world.facts).await;
    assert_eq!(replayed.unwrap(), Replay::Matches);
    let now = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;
    assert_eq!(now.outcome, Outcome::Denied);
}

#[tokio::test]
async fn replay_differs_when_a_namespace_changes() {
    let mut world = world();
    let decision = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;

    // Namespaces aren't versioned yet, so this edit reaches the past revision.
    declare(
        &mut world.namespaces,
        &world.dictionary,
        "file",
        "viewer",
        This,
    )
    .unwrap();

    let replayed = replay(&decision, &world.namespaces, &world.facts).await;
    let Replay::Differs { now } = replayed.unwrap() else {
        panic!("expected the replay to differ");
    };
    assert_eq!(now.outcome, Outcome::Denied);
}

#[tokio::test]
async fn replay_reports_the_same_verdict_on_other_grounds() {
    let world = world();
    let healthy = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;
    // As if a failed read had sent the walk down another path.
    let mut decision = healthy.clone();
    decision.outcome = Outcome::Allowed {
        grounds: grounds(&healthy)[1..].to_vec(),
    };
    let replayed = replay(&decision, &world.namespaces, &world.facts).await;
    assert_eq!(replayed.unwrap(), Replay::Regrounded { now: healthy });
}

#[tokio::test]
async fn replay_reports_a_semantics_change() {
    let world = world();
    let mut decision = decide(&world, "file:design.md#viewer", "alice", head(&world).await).await;
    decision.semantics = Semantics(0);
    let replayed = replay(&decision, &world.namespaces, &world.facts).await;
    assert!(matches!(
        replayed.unwrap(),
        Replay::SemanticsChanged {
            recorded: Semantics(0),
            ..
        }
    ));
}

fn context(request_id: &str) -> RequestContext {
    RequestContext {
        request_id: request_id.to_owned(),
        caller: "docs-service".to_owned(),
        received_at: SystemTime::UNIX_EPOCH,
    }
}

fn request(resource: &str, relation: &str, subject: SubjectName) -> CheckRequest {
    CheckRequest {
        set: format!("{resource}#{relation}").parse().unwrap(),
        subject,
        zookie: None,
    }
}

fn person(name: &str) -> SubjectName {
    SubjectName::Identity(name.parse().unwrap())
}

#[tokio::test]
async fn check_records_each_verdict_with_its_context() {
    let world = world();
    let audit = MemoryAuditLog::default();
    let service = CheckService::new(&world.dictionary, &world.namespaces, &world.facts, &audit);

    let allowed = request("file:design.md", "viewer", person("alice"));
    let verdict = service.check(context("r1"), allowed.clone()).await.unwrap();
    let Verdict::Decided(decision) = &verdict else {
        panic!("expected a decision");
    };
    assert!(decision.outcome.is_allowed());
    assert_eq!(decision.revision, head(&world).await);

    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].context, context("r1"));
    assert_eq!(records[0].request, allowed);
    assert_eq!(records[0].verdict, verdict);
}

#[tokio::test]
async fn check_records_unknown_names_and_failures() {
    let world = world();
    let audit = MemoryAuditLog::default();
    let service = CheckService::new(&world.dictionary, &world.namespaces, &world.facts, &audit);

    // Only namespace and relation names are interned, so an unknown one
    // ends the check before the fact store is read.
    let unknown = request("drive:design.md", "viewer", person("alice"));
    let verdict = service.check(context("r1"), unknown).await.unwrap();
    assert_eq!(verdict, Verdict::Unknown);

    let undeclared = request("file:design.md", "reader", person("alice"));
    let verdict = service.check(context("r1"), undeclared).await.unwrap();
    assert_eq!(verdict, Verdict::Unknown);

    // An identity or resource no fact names is judged, and denied.
    for (resource, who) in [("file:design.md", "mallory"), ("file:nowhere.md", "alice")] {
        let request = request(resource, "viewer", person(who));
        let verdict = service.check(context("r1"), request).await.unwrap();
        assert!(
            matches!(&verdict, Verdict::Decided(decision) if !decision.outcome.is_allowed()),
            "{verdict:?}"
        );
    }

    let mut ahead = request("file:design.md", "viewer", person("alice"));
    ahead.zookie = Some(Revision(head(&world).await.0 + 1));
    let verdict = service.check(context("r2"), ahead).await.unwrap();
    assert!(matches!(verdict, Verdict::Failed(_)));

    let verdicts: Vec<_> = audit.records().into_iter().map(|r| r.verdict).collect();
    assert_eq!(verdicts.len(), 5);
    assert_eq!(verdicts[..2], [Verdict::Unknown, Verdict::Unknown]);
    assert_eq!(verdicts[4], verdict);
}

#[tokio::test]
async fn check_accepts_a_zookie_at_the_latest_revision() {
    let world = world();
    let audit = MemoryAuditLog::default();
    let service = CheckService::new(&world.dictionary, &world.namespaces, &world.facts, &audit);

    let mut current = request("file:design.md", "viewer", person("alice"));
    current.zookie = Some(head(&world).await);
    let verdict = service.check(context("r1"), current).await.unwrap();
    assert!(matches!(verdict, Verdict::Decided(d) if d.outcome.is_allowed()));
}

#[tokio::test]
async fn check_accepts_a_subjectset_as_the_subject() {
    let world = world();
    let audit = MemoryAuditLog::default();
    let service = CheckService::new(&world.dictionary, &world.namespaces, &world.facts, &audit);

    let eng = SubjectName::Subjectset("group:eng#member".parse().unwrap());
    let verdict = service
        .check(context("r1"), request("folder:root", "viewer", eng))
        .await
        .unwrap();
    assert!(matches!(verdict, Verdict::Decided(d) if d.outcome.is_allowed()));
}
