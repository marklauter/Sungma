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

Zanzibar trusts every service that calls Check to name the right [[subject]]. A service that passes a user-supplied identity straight through lets a user claim to be anyone. Sungma closes that gap by never taking a subject by name. This belongs to the roadmap's authentication and authorization of Sungma's own callers.

## Two tokens

- **The caller token** identifies the client calling Check, such as a client-credentials JWT. Sungma authorizes the client in its own model, for example which clients may check which namespaces.
- **The subject token** is the token of whoever the check is about. Sungma validates it and takes the subject from its `sub` claim, so a caller can't ask about Alice without a token Alice was issued. A client acting for a user forwards the user's token. A client acting for itself, such as a batch job, is its own subject and sends its own token as both.

## The subject claim

Sungma trusts one issuer, and a token's subject is that issuer's `sub` claim, as RFC 7519 defines it. This holds for both tokens, so the caller is the caller token's `sub`. A token without `sub` is refused.

People and clients are both subjects, and Sungma doesn't tell them apart. A client in the client-credentials flow gets roles through the same facts as a person.

The issuer is configured so every token it issues carries a `sub` that is stable and unique for each principal. A UPN or an email doesn't qualify: either can change, or pass to someone else, and the facts about the old value would follow. With Rauthy, client-credentials tokens carry `sub` only with `client_credentials_map_sub` turned on.

## One issuer of record

Sungma trusts one identity provider, its issuer of record. A service may trust any identity provider it likes, but to call Sungma it exchanges its token for one from Sungma's issuer, through OAuth 2.0 Token Exchange (RFC 8693). The calling service exchanges the token itself, or a reverse proxy in front of Sungma exchanges it for every caller. Trust between identity providers is configured in the identity providers, so swapping one token for another stays their job, and Sungma never maps another provider's claims.

Two other designs were weighed. Trusting many issuers, each prefixed onto its subjects, needs no exchange, but one person becomes a different subject at each provider, and Sungma would map every issuer's claims. Adding a Sungma person that each provider's accounts link to fixes the first cost, but makes Sungma keep a store of people and link accounts safely, which is an identity provider's job.

## On behalf of

One hop needs nothing more. When service A calls Check with Alice's token, the two tokens already say "Alice, via A".

A longer chain loses the middle. When A calls B and B calls Check, Sungma sees B as the caller and Alice as the subject, and A is invisible. Recording A takes OAuth 2.0 Token Exchange (RFC 8693): the identity server issues B a token that keeps Alice in `sub` and adds an `act` claim naming A. Only the identity server can add `act`, since a PEP can't change a signed token.

When every service shares one identity server, A can forward Alice's token to B unchanged. That skips the exchange, at two costs:

- The token must be valid at both A and B, so a leak at either works at both. An exchanged token can be limited to B.
- B can't tell that A was in the middle.

Forwarding can come first and exchange later without changing the check request, because the subject comes from `sub` either way.

## Open

- How Sungma models which clients may check which namespaces, and which may act for users.
- Whether a check against an unknown namespace or relation answers `Unknown`, which shows a prober the names that exist, or `Denied`.
