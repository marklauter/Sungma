---
title: SCIM
type: note
summary: Sungma owns no users, but Author needs them to search and bind; an organization's identity provider pushes its users and groups to an endpoint Sungma exposes, over SCIM, instead of Sungma calling each vendor's API.
status: evolving
cites:
  - "[[identity]]"
  - "[[fact]]"
---

# SCIM

## Why this note exists

Sungma owns no user pool. An [[identity]] is an opaque key it never looks up. But a person in Author adding alice to a group has to find alice first, so Author needs the organization's users and groups.

Those live in the organization's identity provider, such as Entra ID or Okta. Reading them through each vendor's API means building and keeping a client per vendor, each with its own sign-in, paging and rate limits. SCIM turns that around: the identity provider pushes its users and groups to Sungma, through one standard.

## SCIM in brief

SCIM 2.0 (RFC 7643 for the shapes, RFC 7644 for the protocol) is a standard for provisioning users and groups over HTTP. The app exposes the endpoint, and the identity provider calls it:

- `POST /Users` when someone is assigned to the app, and `PATCH /Users/{id}` when their details change. `active: false` deactivates them.
- `POST /Groups` and `PATCH /Groups/{id}` with members added or removed.
- `GET /Users?filter=...` to find whether someone already exists.

The organization configures it once in its identity provider: Sungma's SCIM URL and a secret token, the attribute mappings, and which users and groups are assigned to the app. Only those are pushed. Okta pushes within moments of a change; Entra runs sync cycles, about every 40 minutes.

## How Sungma uses it

- **Users** land in Author's directory, so search reads Sungma's own copy, with no vendor call at search time.
- **Groups** become [[fact]]s, such as `group:eng#member` holding each member, written in batches through Write.
- **Deactivation** stages removing the person from everything.
- **The identity stored in facts** is the same value Check sees in a token at request time, such as Entra's object id or the OIDC `sub`. SCIM's `externalId` carries it; if the two disagree, checks don't match the users SCIM created.

## Missed updates

SCIM syncs state, not events: each call says what a user or group is now, so Sungma applies a repeated or late call without harm. Providers retry failed calls, and an admin can restart provisioning to send everything again.

What remains is drift: a change lost on either side, so Sungma keeps a membership the directory removed. The identity provider still guards sign-in, so a disabled person can't get a new token to use what they keep; tokens already issued and accounts that don't sign in through it are the exceptions. A full resend on a schedule catches the drift. After it, Sungma removes anyone it holds that the resend didn't include.

## Open

1. Where the SCIM endpoint lives: Write, Author, or a service of its own.
2. Vendor API clients, through their OpenAPI descriptions, only for a provider that can't do SCIM.
