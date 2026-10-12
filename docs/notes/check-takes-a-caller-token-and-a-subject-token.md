---
title: Check takes a caller token and a subject token
type: note
summary: "A check carries two validated tokens, the caller's and the subject's, so no caller can ask on behalf of someone it isn't holding a token for."
status: evolving
cites:
  - "[[subject]]"
  - "[[identity]]"
---

# Check takes a caller token and a subject token

Zanzibar trusts every service that calls Check to name the right [[subject]]. A service that passes a user-supplied identity straight through lets a user claim to be anyone. Sungma closes that gap by never taking a subject by name. This belongs to roadmap item 15, authentication of Sungma's callers.

## Two tokens

- **The caller token** identifies the client calling Check, such as a client-credentials JWT. Sungma authorizes the client in its own model, for example which clients may check which namespaces.
- **The subject token** is the token of whoever the check is about. Sungma validates it and takes the subject from its `sub` claim, so a caller can't ask about Alice without a token Alice was issued. A client acting for a user forwards the user's token. A client acting for itself, such as a batch job, is its own subject and sends its own token as both.

## On behalf of

One hop needs nothing more. When service A calls Check with Alice's token, the two tokens already say "Alice, via A".

A longer chain loses the middle. When A calls B and B calls Check, Sungma sees B as the caller and Alice as the subject, and A is invisible. Recording A takes OAuth 2.0 Token Exchange (RFC 8693): the identity server issues B a token that keeps Alice in `sub` and adds an `act` claim naming A. Only the identity server can add `act`, since a PEP can't change a signed token.

When every service shares one identity server, A can forward Alice's token to B unchanged. That skips the exchange, at two costs:

- The token must be valid at both A and B, so a leak at either works at both. An exchanged token can be limited to B.
- B can't tell that A was in the middle.

Forwarding can come first and exchange later without changing the check request, because the subject comes from `sub` either way.

## Open

- Which claim identifies the caller: `client_id`, `azp`, or `sub` of the caller token.
- How Sungma models which clients may check which namespaces, and which may act for users.
- Whether a check against an unknown namespace or relation answers `Unknown`, which shows a prober the names that exist, or `Denied`.
