---
title: Sungma is single-tenant
type: note
summary: One Sungma deployment serves one organization and trusts one identity provider; an organization runs it in its own cloud, or a vendor runs an isolated instance for it as single-tenant SaaS, and the code is the same either way.
status: evolving
cites:
  - "[[check-takes-a-caller-token-and-a-subject-token]]"
---

# Sungma is single-tenant

## The decision

A Sungma deployment serves one organization. Its services, its table and its timeline belong to that organization alone. No key carries a tenant, and no check, read or write chooses between organizations.

An organization gets Sungma one of two ways:

1. **Self-hosted.** The organization runs Sungma in its own cloud.
2. **Single-tenant SaaS.** A vendor runs an isolated instance for the organization: its own services and its own table, ideally in an account of its own. The vendor deploys, upgrades and monitors it, and the organization subscribes. It's still SaaS; only the hosting is isolated.

The code is the same either way. What differs is who runs it.

## Why

Sungma holds a map of who can reach what across a whole organization. A security-minded organization won't keep that map in a table shared with other organizations, separated only by a key and the vendor's care. A separate instance gives it isolation it can see.

Single tenancy also keeps the design as it is. A multi-tenant Sungma would put a tenant into every key, keep name pools and namespaces per tenant, and need a head and a revision timeline per tenant, so one tenant's writes don't contend with another's.

## Identity

A deployment trusts one issuer: the organization's identity provider. Read, Write and Check accept tokens only from it, with the audience it issues for Sungma's APIs. Sungma doesn't run an identity provider of its own.

An organization whose provider can't issue tokens for Sungma's APIs, or that signs people in through several providers, runs a broker, such as Keycloak, Dex or Ory, as the one issuer, and the broker passes sign-in on to the others. The deployment still trusts one issuer.

## What it costs

- **A deployment per organization.** Onboarding has to be automated, such as one template per instance.
- **The cost of a quiet organization.** DynamoDB on demand costs little at rest, so the cost of an idle instance is mostly its services.
- **No shared tier.** A cheap tier for small organizations, all in one deployment, isn't offered. If one is ever wanted, it is a separate offering, and it doesn't shape Sungma's design.
