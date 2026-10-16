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
10. Watch service: a changelog stream with heartbeat zookies.
11. Namespace documents: the JSON form of a namespace, with its rewrite grammar, parser and printer (done).
12. Versioned namespaces (done) and a namespace write service.
13. HTTP and gRPC transports for each service.
14. Error kinds: every domain error carries a kind, such as validation, not found, gone, conflict or unavailable, and a stable code, such as `fact.undeclared_relation`. Each transport maps kinds to its own status codes in one table.
15. Authentication and authorization of Sungma's own callers.
16. Distributed storage adapter.
17. Multi-region deployment with snapshot reads from local replicas.
18. Garbage collection of fact history past a retention window.
19. Check result caching.
20. Index for large and deeply nested groups.
21. Snapshot dumps for offline processing.
22. Metrics, tracing and latency objectives.
23. Client SDKs.
24. Opaque zookies.

## Deferred

- ClockBound clock: AWS ClockBound's microsecond bounds, as a `SystemClock` variant. Needs a host to run the daemon on, so it waits for an EC2-backed deployment; Fargate has none.
- Clock soak test: a long run comparing the clock's readings against servers it doesn't sync from, counting readings that miss them. Deferred until there's a reason to gather the evidence.
