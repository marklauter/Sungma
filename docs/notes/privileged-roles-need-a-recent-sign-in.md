---
title: Privileged roles need a recent sign-in
type: note
summary: "A person holding a privileged role can use it only with a recent sign-in, so a session doing everyday work can't change the authorization model or who holds roles."
status: evolving
cites:
  - "[[sungma-authorization]]"
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
  - "[[author-needs-an-oauth-bff-crate]]"
---

# Privileged roles need a recent sign-in

## The need

Someone who holds a powerful role holds it all the time, including while they do everyday work. A stolen session, or a slip of the mouse, then reaches everything the role allows. An operating system handles this with elevation: a person works as a plain user and confirms who they are before an action that needs more.

Sungma does the same. Signing in gives everyday work. Using a privileged role takes a fresh sign-in.

## Privileged roles

These roles in [[sungma-authorization]] change the authorization model itself, or who holds other roles:

- On the deployment: `admin` and `namespace_creator`.
- On a namespace: `owner` and `writer`.
- On a group: `owner`.

Writing and reading facts, reading namespace documents, and the `operator` and `auditor` roles are everyday work. They don't need elevation.

## Step-up sign-in

OpenID Connect already carries what elevation needs:

- A token's `auth_time` claim says when the person last signed in.
- An authorization request's `max_age` parameter makes the identity provider ask the person to sign in again when their last sign-in is older than that.

So:

1. Sungma accepts a privileged operation only from a caller token whose `auth_time` falls within the elevation window.
2. When Author is about to do one and the token is older, Author's BFF ([[author-needs-an-oauth-bff-crate]]) sends the person through sign-in again with a short `max_age`. The person sees a prompt, as with an elevation prompt, and carries on with a fresh token.

Nothing in the model changes. A person still holds `admin`, and Sungma honors it only with a recent sign-in.

A client in the client-credentials flow gets a fresh token each time and has no person to prompt. A client that needs a privileged role is a client of its own, kept apart from the clients that do everyday work.

## Eligible and active roles

A stronger form keeps privileged roles off a person until they ask: they hold `admin_eligible`, and elevating writes an `admin` fact that expires after a set time. That needs facts that expire on their own, which Sungma doesn't have.

## Open

1. How long the elevation window is, and whether it differs by role.
2. Whether Rauthy honors `max_age` and reports `auth_time` as OpenID Connect specifies.
3. Whether Sungma should have facts that expire, which eligible and active roles need.
4. How a privileged client is told apart from an everyday one: its own client, or a scope it must ask for.
