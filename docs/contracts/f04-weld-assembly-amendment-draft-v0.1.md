# Weld amendment draft: a workshop weld builds an assembly

Status: **draft for owner review (2026-10-23).** The vocabulary and F04 0.1.0 stay unchanged until
the owner accepts this text. It extends the minimal weld of [A06](../design/a06-welds.md), which
only links two parts, with one behaviour: a **workshop weld joins the two parts into one
assembly**. It is the contract for the slice A06.2 in the [GitHub task catalogue](https://github.com/Apibim-com/BeamCraft/issues/474); the weld
geometry, length and symbols stay in A15.

Interpretation: "workshop" is the `workshop` value of `steel:weld:location`. A site weld joins
parts on the building site, after the shop assemblies exist, so it changes no assembly.

## 1. Rule

A weld names two parts in order. The **first** part is the **main** part; the **second** is welded
to it. Creating a weld with `location = workshop` also makes them one assembly, in the same
transaction and the same undo unit as the weld:

| State before | Result |
|--------------|--------|
| Neither part is in an assembly | A new assembly is created: its name is the name of the main part, its main part is the first part, and both parts are attached to it |
| The first part is in an assembly | The second part is attached to that assembly; the main part of that assembly does not change |
| Both parts are in the same assembly | Only the weld is created |
| The second part is in an assembly and the first is not, or the parts are in two different assemblies | `DEPENDENCY_CONFLICT` naming the assembly; nothing is created. A part belongs to one assembly (A01, `DV-REL-004`) and is never moved silently; the user detaches first |
| `location = site` | Only the weld is created |

A group of welds builds one assembly step by step: weld 1 joins A and B (A is main), weld 2 joins
A and C and attaches C to the assembly of A, and so on.

## 2. Consequences

* The weld still records its two parts in order (`bim:connects`, main part first). The assembly
  is a separate element, as in A01; the parts attach to it with their `part-of` relationship.
* **The weld now writes the parts**: each attached part gets a `part-of` record, as in A01, where
  A06 only read the parts. The weld's footprint declares those writes, so a concurrent edit of a
  part's assembly membership conflicts as for any attach.
* **Deleting the weld keeps the assembly.** The user detaches or deletes it explicitly; the engine
  never dissolves an assembly as a side effect. Changing a weld from workshop to site does not
  change the assembly either.
* **Undo** removes the weld and whatever assembly and memberships the same command created, as one
  unit.
* A refused assembly step (a conflict, a cycle, a missing assembly) refuses the whole command and
  changes nothing.
* The assembly name can be changed later; the numbering and marks of assemblies stay in A08 and
  A09.

## 3. Command

`CreateWeld` gets one optional argument, `assemble`: `auto` (default: a workshop weld joins the
parts, a site weld does not) or `off` (never touch assemblies). CLI: `create-weld ... [--assemble
auto|off]`. `off` is a small addition of this draft and may be dropped.

## 4. Needs

The first part is the **main part** of the assembly, which A01 does not have (no main part yet).
A16.1 adds it: the main part of an assembly, stored on the assembly, one per assembly, always one
of its parts. A06.2 depends on A16.1.

## 5. Acceptance cases (to be bound to fixtures)

Positive:

1. A workshop weld of two free parts creates one assembly with the first part as main and both
   parts attached; the assembly is named after the main part; one undo unit removes all.
2. A second workshop weld of the main part and a third part attaches the third part to the same
   assembly; the main part is unchanged.
3. A weld of two parts already in one assembly creates only the weld.
4. A site weld of two free parts creates no assembly.
5. Deleting a workshop weld leaves the assembly and its members.

Negative (each changes nothing):

1. The second part is in an assembly and the first is not.
2. The parts are in two different assemblies.
3. A concurrent change of a part's membership between the read and the commit.

## 6. Open question

* **O-W1.** When the second part already belongs to another assembly (the common shop case of
  welding a finished sub-assembly to a main part), should that whole assembly become a
  sub-assembly of the first part's assembly instead of a conflict? The draft refuses it for now,
  as A01 never re-parents silently; nested assemblies are A16.

## 7. Provenance

The behaviour is the owner's description of 2026-10-23; the rules and wording are our own and use
only the assembly and weld models already in this repository.
