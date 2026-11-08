---
title: A retired namespace can be restored
type: note
summary: "Sungma keeps every namespace version, epoch and fact span, so a retired namespace or a dropped relation can be restored with its facts by an ordinary write that brings its old epochs back, rather than by restoring the database."
status: evolving
cites:
  - "[[namespace]]"
  - "[[relation]]"
  - "[[epoch]]"
  - "[[span]]"
  - "[[fact]]"
  - "[[sungma-authorization]]"
  - "[[privileged-roles-need-a-recent-sign-in]]"
---

# A retired namespace can be restored

People retire namespaces by accident. Retiring the namespace that controls access to an important system cuts everyone off at once, and a point-in-time restore of the whole DynamoDB table would also roll back every other change since.

Sungma doesn't need that, because it never throws history away. A retired [[namespace]]'s versions and [[epoch]]s are still stored, and its facts' [[span]]s are never removed. Retirement only stops them counting. So restoring is an ordinary write.

## Restore is not recreate

- **Recreating** `billing` gives every [[relation]] a new epoch. Facts from before have spans that opened before those epochs, so they stay dead. The namespace starts fresh.
- **Restoring** `billing` writes a version equal to its last version before retirement, and keeps that version's epochs. Every old [[fact]] whose span is still open counts again. Nothing is copied or touched: the spans were there all along, and the epochs decide again that they count.

No fact can be written under a retired namespace, so nothing new gets mixed in.

The same works one level down. Restoring a relation that a version dropped keeps its old epoch, so its facts come back too.

## It fits names and creations

Sungma's own facts are about a creation of a namespace, not its name, as [[sungma-authorization]] describes under names and creations. Restore resumes the old creation, such as `billing@40`, so the facts about who managed it apply again. Recreate starts a new creation, which no old fact mentions. The difference between restore and recreate is exactly whether the past comes back.

Facts in other namespaces that point into the restored one come back too, if they record their subject's epoch as the authorization spec proposes. A fact such as `file:secret#viewer@billing:acct-42#member` recorded the old epoch, so it counts again once that epoch is back.

## Restoring facts

The same history undoes a mistake in facts. A retired fact keeps its span, closed at the revision that retired it. To bring back facts retired by mistake, such as a bulk retire, Sungma reads them as they stood at a revision before the mistake and writes them again. The restored facts open new spans, so the history keeps both the mistake and its undoing.

Undo is the guard against mistakes, rather than asking people to confirm destructive actions or splitting adding facts from retiring them.

## Who may restore

The namespace's owners, or admins, with a recent sign-in as [[privileged-roles-need-a-recent-sign-in]] describes. Restoring brings back every old grant on purpose, so it's privileged, like retiring.

## Open

1. **A name created again since.** Restore works while nothing has replaced the name. If someone recreated `billing` since, the old creation still exists, but a name has only one current creation. Either the restore is refused, or the new creation must be retired first.
2. **Restoring to an earlier version.** Whether restore only brings back the last version before retirement, or any earlier version with the epochs it had then.
3. **Facts retired before the retirement.** Restore brings back what counted at the moment of retirement. Facts retired earlier stay retired, which matches the timeline, but the spec should say so plainly.
