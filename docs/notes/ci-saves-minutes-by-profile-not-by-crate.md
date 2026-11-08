---
title: CI saves minutes by profile, not by crate
type: note
summary: "Running only the crates a change affects has too many ways to run too little, so pull requests run the dev-profile tests on Ubuntu and Windows, and pushes to main add the release-profile tests. That saves the most minutes and can't skip a crate."
status: evolving
---

# CI saves minutes by profile, not by crate

GitHub Actions runner minutes are limited, so CI should spend them where a run can't miss anything.

## Narrowing to affected crates

The obvious saving is to work out which crates a change touches, add every crate that depends on them, and test only those. A script can read the crate graph from `cargo metadata`. It runs too little whenever its picture of the change is wrong, and the change has many ways to be wrong:

- **Files outside any crate.** `sungma-author` compiles in `assets/logo/sungma.png` with `include_bytes!`, and `.gitattributes` sets the line endings of every fixture. Neither belongs to a crate.
- **Files read across crates.** `sungma-lang`, `sungma-check` and `sungma-write` read `crates/sungma/tests/fixtures`, as does the `fact` fuzz target.
- **Moved files.** `git diff` reports a move at its new path only, so the crate a file left looks untouched.
- **Non-ASCII paths.** Git quotes them by default, so they match no crate.
- **Feature unification.** A `-p` subset unifies Cargo features differently from the workspace, so a crate that relies on a feature another crate turns on passes alone and fails together.
- **Cancelled runs.** With `cancel-in-progress`, a cancelled run on main leaves its crates untested, because the next run diffs only against the run before it.
- **Shell pipes.** `grep -q` closing a pipe under `pipefail` reads as "no match", which narrows a run that should run everything.

Each fix adds code, and that code has edges of its own.

## Where the minutes go

One full run on a pull request:

| Job | Minutes |
|---|---|
| Tests, Windows release | 8.5 |
| Tests, Ubuntu release | 5.3 |
| Tests, Windows dev | 4.4 |
| Tests, Ubuntu dev | 1.7 |
| CodeQL | 7.5 |
| Fuzz | 5.4 |
| Mutants, four shards | about 2 each |
| Coverage | 2.2 |

The release-profile tests are about 14 of those minutes.

## What CI does instead

1. **Pull requests run the dev-profile tests only,** on Ubuntu and Windows. Pushes to main also run the release-profile tests. That saves about 14 minutes on every pull request, and it can't skip a crate.
2. **Everything else stays as it is.** One yes-or-no gate skips Rust jobs for changes with no Rust in them, jobs run on the whole workspace, and mutants already test only the diff.

A release-only failure then shows up on main rather than on its pull request. The release profile turns on optimization and turns off debug assertions and overflow checks, so a release-only failure comes from one of those, and is rare.

## Open

1. Whether fuzzing on pull requests can be shorter, with the full minute on main.
