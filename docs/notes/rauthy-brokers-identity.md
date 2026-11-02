---
title: Rauthy brokers identity
type: note
summary: Every SaaS needs an identity provider, so a Sungma deployment ships Rauthy as its one issuer; Rauthy federates the organization's own providers, and Sungma keeps only a subject and what its UI shows.
status: evolving
cites:
  - "[[sungma-is-single-tenant]]"
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
  - "[[author-needs-an-oauth-bff-crate]]"
  - "[[subject]]"
  - "[[identity]]"
---

# Rauthy brokers identity

## The decision

A Sungma deployment ships Rauthy as its identity provider. Rauthy is the one issuer that [[sungma-is-single-tenant]] calls for. It federates the organization's own providers, such as Okta, Entra or Auth0, over OIDC, and issues the tokens Read, Write, Check and Author accept.

This changes one line of [[sungma-is-single-tenant]]: Sungma no longer leaves the broker to the organization. An identity provider is table stakes for a SaaS, and Sungma won't implement OAuth itself.

Sungma keeps no user store of its own beyond:

- **The subject:** Rauthy's `sub`, an opaque, stable user ID. Tuples name subjects by it.
- **A display projection:** name and email for Author, a cache fed by SCIM, never a source of truth.

## Why Rauthy

- **Rust, one binary, Apache-2.0.** It runs beside Sungma's services and can be wrapped, with its admin API driven by Sungma's control plane.
- **A full OAuth 2 and OIDC provider:** authorization code with PKCE, client credentials, device code, dynamic client registration, back-channel logout, DPoP, and automatic JWKS rotation. New clients default to ed25519 and S256 PKCE.
- **Federation:** upstream OIDC providers, with JIT onboarding (`auto_onboarding`) and email linking (`auto_link`).
- **SCIM v2 to downstream clients,** so Sungma learns about users and groups without reading Rauthy's database.
- **An independent security audit,** with its findings fixed in v0.32.1.
- **Small:** about 40 MB per node on Hiqlite.

## Alternatives

- **Keycloak:** has SAML, realms, LDAP federation, and many linked identities per user. But it's Java, heavy to run, and extended through Java SPIs.
- **Ory Hydra with Kratos or a user context of Sungma's own:** Hydra handles the protocol and delegates login and consent, the closest to Duende. But it's Go, and the user side has to be run or built.
- **No broker, trusting each organization's provider directly:** the least infrastructure. But it means N claim shapes and no common subject, and it reverses [[sungma-is-single-tenant]]'s one issuer.

## How it fits

### Tenancy

Single tenancy removes Rauthy's biggest gap. Rauthy has no realms, and Sungma needs none: one deployment means one Rauthy, one JWKS, and one user namespace.

### Users and groups reach Sungma through SCIM

Sungma implements a SCIM v2 endpoint, and Rauthy pushes user and group changes to it. Sungma learns about users and groups only through this endpoint. It doesn't read them from Rauthy's API, so it stays independent of the IdP, and development uses the same path as production. Rauthy's admin API is for seeding test users, not for feeding Sungma. The endpoint is the anti-corruption layer: it turns SCIM resources into tuples such as `group:eng#member@user:<sub>` and refreshes the display projection.

Rauthy's event stream doesn't carry group or role changes. Its events cover security and operations: invalid logins, brute force, new users, email changes, password resets, JWKS rotation and health. Sungma may subscribe for side signals, such as ending sessions after `UserPasswordReset`. The stream is SSE without a documented resume cursor, so a consumer reconnects and fills gaps from `/auth/v1/events`.

Watch is separate. It streams Sungma's own tuple changelog, ordered by its timeline, and never depends on Rauthy's delivery.

In development, Rauthy runs in a container, so the SCIM base URL has to name the host as the container sees it, such as `host.containers.internal`. `localhost` would point back at Rauthy's own container.

### Federated identities

The general model keys external identities by `(iss, sub)`, never by email. A local account has many identities, and email links a new one only when `email_verified` holds and the issuer is trusted for the domain.

Rauthy holds the same shape with cardinality one: `auth_provider_id` and `federation_uid` sit on the user, so an account binds to one upstream provider.

- A login from a second provider with the same email is refused, or with `auto_link` it moves the binding. It doesn't add a second identity.
- `auto_link` is on only for a provider authoritative for the email's domain, such as the organization's corporate provider. It's never on for social providers.
- People with different emails are different subjects. Sungma doesn't merge them. If merging is ever wanted, it's a relation in Sungma, such as `user:a#same_as@user:b`, which is auditable and reversible.

### High availability

Rauthy's HA runs on Hiqlite's Raft, and some state, such as authorization codes, lives only in the Raft-replicated cache, even on Postgres. So:

- **Default:** a three-node cluster across availability zones in one region. Hiqlite backups go to S3 with cross-region replication, and Rauthy restores from S3 in the standby region behind Route 53 failover. Recovery restores users, clients and signing keys, so issued tokens still validate. The recovery point is the backup interval, and sessions in flight are lost.
- **Only if near-zero recovery time is required:** a Raft cluster stretched across three nearby regions. Every login step pays a cross-region commit. Measure it first.
- **Not viable:** independent clusters, active-active. Nothing syncs them, so users, clients and keys diverge.

Hiqlite on its own, not Postgres or Aurora Global under a Raft cache, keeps one consistency domain.

## What it costs

- **No SAML.** An organization that only speaks SAML needs a SAML-to-OIDC bridge in front.
- **One upstream provider per user.** People who switch between, say, Okta and Auth0 can't use both on one account. Moving federation into a `user_federations` table keyed by `(provider, federation_uid)` would be a contained upstream change.
- **Offboarding lags.** Rauthy pushes SCIM but doesn't take it in, so a user disabled upstream stays live until their next upstream sign-in. Short sessions, short tokens, and an offboarding call from the control plane narrow the gap.
- **Pre-1.0, and maintained mostly by one person.** Sungma pins Rauthy's version, tests upgrades, and keeps its facade thin so Rauthy can be replaced.
- **Customization is config, events and API.** It has no plugin interface, so custom grants or login logic would mean a fork or a different broker.

## Open

1. Whether Rauthy maps upstream group claims into its own groups, or whether groups are managed in Rauthy or Sungma.
2. How the control plane provisions a deployment's Rauthy: clients, scopes, upstream providers and API keys.
3. What the SCIM endpoint maps: users and groups only, or custom attributes to relations too.
4. Whether Author's BFF ([[author-needs-an-oauth-bff-crate]]) gets its tests from a fake provider alone, or also from a Rauthy container in an integration suite.
