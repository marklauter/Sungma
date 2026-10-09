# Roadmap

1. Evaluator: theories, facts and rewrites, judged at a revision (done).
2. Decisions with grounds, audit records and replay (done).
3. Storage ports with Sungma-owned fact history and pooled id minting (done).
4. SQLite storage adapter for development.
5. Opaque zookies.
6. Check service.
7. Content-change check that returns a zookie.
8. Read service: facts by key, resource or subjectset at one snapshot.
9. Write service: atomic batches with conditional writes.
10. Expand service.
11. Watch service: a changelog stream with heartbeat zookies.
12. Theory documents: the JSON form of a theory, with its rewrite grammar, parser and printer (done).
13. Versioned theories and a theory write service.
14. HTTP and gRPC transports for each service.
15. Authentication and authorization of Sungma's own callers.
16. Distributed storage adapter.
17. Multi-region deployment with snapshot reads from local replicas.
18. Garbage collection of fact history past a retention window.
19. Check result caching.
20. Index for large and deeply nested groups.
21. Snapshot dumps for offline processing.
22. Metrics, tracing and latency objectives.
23. Client SDKs.

## Deferred

- ClockBound clock: AWS ClockBound's microsecond bounds, as a `SystemClock` variant. Needs a host to run the daemon on, so it waits for an EC2-backed deployment; Fargate has none.
- Clock soak test: a long run comparing the clock's readings against servers it doesn't sync from, counting readings that miss them. Deferred until there's a reason to gather the evidence.
