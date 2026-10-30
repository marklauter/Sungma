---
title: Author UI ideas
type: note
summary: Ideas for Author's screens, captured as they come, before each settles into a note of its own.
status: evolving
---

# Author UI ideas

The wireframes are on the Author canvas in Claude Design: https://claude.ai/artifact/DFAXkfkCrLtvjCUx1JH2V3. Their source is in [wireframes](wireframes), one file per screen and the canvas's layout; they render only on the canvas.

## What people come to do

Most visits are by people who manage users, for one of four jobs:

1. **Access:** grant or revoke it, by adding or retiring relationships. Taking someone out of everything at once, when they leave, stages a retire for each of their facts into one batch, so one write removes them everywhere at one revision.
2. **Answer why:** troubleshoot, diagnose, explain or verify a yes or a no, with the facts behind it. For an auditor, prove a past decision: replay it and show its grounds.
3. **View access:** someone's access, your own first, and everyone who can reach one thing. Someone's access is a read by subject, which Sungma can't do yet; the wireframes show it anyway.
4. **Audit:** what is changing, as a ledger of writes, and what someone could see at a past time, such as yesterday or last week, read at that time's revision.

## Home

Home is mostly information, with links out to the jobs above. Search lives in the masthead, on every page, so Home doesn't need its own.

Each job has a piece of information on Home:

- **You:** your name, title and subject id, with links to your own access and your profile.
- **Access:** a short list of starting points, such as adding a relationship or removing someone from everything.
- **Answer why:** your recent checks, each with its answer, to reopen while working a ticket.
- **Security alerts:** first in the middle column. Heuristics over writes and checks that flag possible attacks: a writer granting itself a relationship, an unusually large grant to a sensitive group or at an unusual hour, and one client checking a great many resources for one subject, most answered no, which looks like someone mapping an account's reach. A namespace version that widens access a lot belongs here too. Password changes and sign-in attacks are the identity provider's to detect; Author could show its alerts through a port later. Shinje, the sister project, a risk-adaptive access control system that weighs each request before judging it, is the natural source of these alerts once the two are integrated.
- **System status:** below the security alerts, at the same width: one line for the services, a service falling behind the head, facts orphaned by a namespace version that dropped their relation, and scheduled maintenance.
- **Audit:** the change feed, the ledger of recent writes.
- **View access:** saved views, searches saved as results that stay as they were until refreshed.
- **Recent:** what you viewed or changed lately, as a vertical list, the way the AWS console's recently visited panel shows it, not a horizontal strip.

Information runs vertically: lists read top to bottom, since eyes move up and down a page more easily than across it.

## Landing page reference

What the Microsoft Entra admin center's home page shows, as a reference:

1. **A "me" panel:** your name, your title (such as Global Administrator), and your subject id, with links to view your roles and your profile.
2. **A tenant panel:** the tenant's name, its id and its primary domain, and four links with counts: users, groups, devices (the phones and computers registered to accounts) and apps.
3. **A tenant status panel:** a security score as a percentage, with a link to ideas for improving it.
4. **Shortcuts:** an "Add" menu (user, guest, group, enterprise app), then sign-in logs, audit logs, authentication methods, blocked users, domain names, unused principals and managing tenants.
5. **Help at the bottom:** a knowledge center, kept out of the way of people who come every day but easy to find. In Author it goes in a panel of its own on the right.

Entra assumes you already know identity and access: someone new to authentication and authorization is lost there. Author is built for people who do this work, not for beginners, but it gives beginners somewhere to go: the help panel explains the ideas behind each screen and links to a worked example.

## Saved views and a query language

A saved view stores its question, its answer, and the revision it was answered at. The question starts as one of Sungma's reads with its arguments, such as expanding `folder:specs#viewer` or reading every fact whose subject is `bob`, with perhaps a filter on the answer. Author shows it as a sentence, such as "Who can view the specs folder".

A query language comes later, if people ask questions that combine reads, such as everyone in eng who can view payroll but isn't in finance. That is joins and set operations across reads: a language of Sungma's own, or one like Cypher or Gremlin, with a parser, a planner and limits on cost. Either way, a question can only ask what Sungma's indexes can answer: today a read by subjectset, and later by subject and by resource.

## Favorites

Pinned items are favorites, reachable from every page, not only from Home.

## Docs an AI can read

Every page of help is also plain Markdown, so an AI assistant can read and navigate it: a path such as `/docs/ai` serves the docs as Markdown files only, with an index of them. The llms.txt convention does the same with a `/llms.txt` index at the site's root, linking a Markdown copy of each page.

## Tenants within an organization

An organization may want tenants of its own: a client with ten major divisions would likely keep each division in a tenant of its own, all within one Sungma deployment.

## Open

1. **Tenants within one deployment.** [Sungma is single-tenant](../../notes/sungma-is-single-tenant.md) says a deployment serves one organization and no key carries a tenant. Divisions as tenants inside one deployment would bring a tenant back into the keys, the name pools and the timeline. Either the note changes, or a division is something lighter than a tenant, or each division gets its own deployment.
