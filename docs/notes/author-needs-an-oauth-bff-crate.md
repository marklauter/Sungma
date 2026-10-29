---
title: Author needs an OAuth BFF crate
type: note
summary: Author, Sungma's UI, runs behind an OAuth backend-for-frontend that holds the tokens on the back channel and renders HTMX pages; Rust has the parts but no BFF library, so Sungma builds one as a crate of its own.
status: evolving
cites:
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
---

# Author needs an OAuth BFF crate

## The shape

Author, Sungma's UI, is a backend-for-frontend (BFF): one service that signs the user in, holds their tokens, calls Read and Write, and renders HTML that HTMX swaps into the page. The browser never sees a token or Sungma's APIs, so Read and Write stay on a private network, serving machines. This follows the BFF pattern in the IETF draft "OAuth 2.0 for Browser-Based Applications".

The BFF calls Read and Write through a client generated from their OpenAPI descriptions, not through Sungma's crates, so it depends on the APIs' contract alone.

## What the crate does

1. **Signs the user in.** The BFF is a confidential OAuth client. It runs the authorization code flow with PKCE, which OAuth 2.1 requires of every client, and checks the ID token.
2. **Keeps the tokens on the server.** The access and refresh tokens live in a server-side session. The browser holds only a session cookie: HttpOnly, Secure, SameSite.
3. **Attaches the access token to outbound calls,** refreshing it over the back channel before it expires.
4. **Signs the user out,** ending the session and calling the identity provider's end-session endpoint.
5. **Refuses cross-site requests.** The cookie goes with every request, so a request that changes state must also carry a custom header. HTMX adds it to every request, and a cross-site form can't send it.

## What exists

- `openidconnect`, on top of `oauth2`: the authorization code flow, PKCE and ID token checks.
- `tower-sessions`: server-side sessions behind a cookie, stored in memory, Redis or SQL.
- `axum-oidc`: OIDC sign-in and sessions for axum, without token refresh for outbound calls.

No Rust crate covers the whole BFF, as Duende's BFF library does for .NET. Sungma writes the rest: refresh before outbound calls, attaching tokens, sign-out at the identity provider, and the header check. That's a few hundred lines over the crates above.

## What it takes

- The crate lives with the other libraries and is held to their bar: its own tests catch its own mutants, as for a library published on crates.io.
- It's security code. Its tests cover a tampered or missing cookie, an expired token refreshed before a call, a failed refresh signing the user out, a state-changing request without the header, and a stolen authorization code that PKCE refuses.
- Its tests run against a fake identity provider in its test kit: a small axum app on a random port, serving the endpoints the BFF calls.
  1. Discovery, `/.well-known/openid-configuration`, pointing at the others.
  2. The signing keys, the public half of a test key.
  3. Authorize, which redirects straight back with a code, with no sign-in page.
  4. Token, which checks the PKCE verifier and trades a code or a refresh token for tokens signed with the test key.
  5. End session, for sign-out.

  Each test sets how the fake misbehaves: an expired token, a signature from the wrong key, a refused refresh. No identity provider runs outside the tests, so the crate owns its tests and its mutants run without a container.
- Read and Write check the access token the BFF forwards. That's roadmap item 15, authentication and authorization of Sungma's own callers; see [[check-takes-a-caller-token-and-a-subject-token]].

## Open

1. **The session store** past one BFF instance: Redis, or the table.
2. **Templates:** maud or askama.
