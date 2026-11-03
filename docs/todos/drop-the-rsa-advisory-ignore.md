---
title: Drop the rsa advisory ignore
type: todo
summary: Remove the RUSTSEC-2023-0071 ignore from `.cargo/audit.toml` once openidconnect depends on an rsa release the advisory lists as patched, or sooner if Sungma ever signs or decrypts with RSA.
priority: low
status: open
cites:
  - "[[author-needs-an-oauth-bff-crate]]"
---

# Drop the rsa advisory ignore

`.cargo/audit.toml` ignores RUSTSEC-2023-0071, the Marvin attack on the `rsa` crate, so `cargo audit --deny warnings` passes in CI. `sungma-bff` pulls `rsa` 0.9 in through `openidconnect` 4.1, which needs it to verify RS256 ID tokens, and doesn't let a user turn it off.

The attack recovers an RSA private key from the timing of decryption or signing. `sungma-bff` only verifies signatures with the identity provider's public keys and holds no RSA private key, so the attack has nothing to recover.

## When to act

1. **When the fix reaches us.** The advisory lists no patched version. Once it lists one, and `openidconnect` depends on it, update both and delete the ignore. The Dependabot alert for GHSA-c38w-74pg-36hr stays open until then, as a reminder.
2. **Before Sungma signs or decrypts with RSA.** Issuing Sungma's own RS256 tokens, for example, would give the attack a private key to recover. Delete the ignore first, and choose an RSA implementation that isn't affected, such as `ring` or `aws-lc-rs`.

## Done when

`.cargo/audit.toml` no longer ignores RUSTSEC-2023-0071, and `cargo audit --deny warnings` passes.
