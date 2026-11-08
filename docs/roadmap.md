# Roadmap

Done means built, tested to the coverage floor and merged. It doesn't mean production ready: use will bring fixes, refactors and discoveries.

1. Evaluator: namespaces, facts and rewrites, judged at a revision (done).
2. Decisions with grounds, audit records and replay (done).
3. Storage ports with Sungma-owned fact history and pooled id minting (done).
4. DynamoDB storage adapter, tested against DynamoDB Local (done).
5. Check service.
6. Content-change check that returns a zookie.
7. Read service: facts by key, resource or subjectset at one snapshot.
8. Write service: atomic batches with conditional writes.
9. Expand service.
10. Listing what a subject can reach: return the resource ids of one namespace that a subject reaches through a relation, a page at a time at one snapshot, by walking the model back from the subject. And check a set of candidate ids in one call, returning the ones the subject reaches. Applications use the ids to load their own objects.
11. Watch service: a changelog stream with heartbeat zookies.
12. Namespace documents: the JSON and TOML forms of a namespace, with its rewrite grammar, parser and printer (done).
13. Accepted subjects: each relation declares the subjects its facts may bind, and namespace writes and fact writes check against them.
14. Conditions: a relation may declare a condition, an expression over named parameters, and a fact may carry it with some parameters filled in. A check supplies the rest as context, and answers allowed, denied, or conditional when context is missing. Uses include time windows such as building access from 7am to 8pm, elevation by requiring a recent `auth_time` for privileged relations, and grants that expire at a set time.
15. Versioned namespaces (done) and a namespace write service.
16. Restoring namespaces, relations and facts: bring back a retired namespace, or a relation a version dropped, with its old epochs, so its facts count again without restoring the database. Bring back facts retired by mistake, such as a bulk retire, by writing again the facts that counted at an earlier revision. See [A retired namespace can be restored](notes/a-retired-namespace-can-be-restored.md).
17. HTTP and gRPC transports for each service.
18. Error kinds: every domain error carries a kind, such as validation, not found, gone, conflict or unavailable, and a stable code, such as `fact.undeclared_relation`. Each transport maps kinds to its own status codes in one table.
19. Authentication and authorization of Sungma's own callers.
20. Rate limiting: each caller gets a request budget per endpoint, and a caller over its budget is refused until the budget refills. Endpoints that only remove access, such as removing a group member or a role, and fact writes that only retire facts, have budgets of their own, so a caller's revocations never wait behind its own adds. Every write competes for the head, so without budgets one caller flooding writes can make everyone else's, revocations included, run out of retries.
21. SQLite storage adapter: run Sungma in one container on a SQLite file, with the same write-id protocol as DynamoDB, as a free, self-hosted edition.
22. Distributed storage adapter.
23. Multi-region deployment with snapshot reads from local replicas.
24. Garbage collection of fact history past a retention window.
25. Check result caching.
26. Index for large and deeply nested groups.
27. Snapshot dumps for offline processing.
28. Metrics, tracing and latency objectives.
29. Client SDKs.
30. Model simulation and review: load a set of namespaces and facts locally, without writing them to a deployment, run Check and Expand against them, and report who can reach what, including paths by which a role reaches more than it was granted.
31. Namespace diagrams: Author draws the model as a graph, with each namespace as a node and each relation's accepted subjects as edges.
32. Patterns wiki: a library of model patterns customers can start from, such as roles at a root that reach every namespace through `parent`, namespaces and their relations, nesting groups, auditors as a kind of reader, and owners deciding who holds what. Sungma's own authorization model is the first worked example. Another entry explains listing what a user can see: ask Sungma for the ids and load your objects by them, or filter your own candidates through a batch check when your own filter is narrower, and how to fill a page when few candidates pass.
33. Token-exchanging proxy: a reverse proxy in front of Sungma that exchanges a caller's token for one from Sungma's issuer of record, so callers that trust other identity providers need no exchange code of their own.
34. Opaque zookies.

## Deferred

- ClockBound clock: AWS ClockBound's microsecond bounds, as a `SystemClock` variant. Needs a host to run the daemon on, so it waits for an EC2-backed deployment; Fargate has none.
- Clock soak test: a long run comparing the clock's readings against servers it doesn't sync from, counting readings that miss them. Deferred until there's a reason to gather the evidence.
