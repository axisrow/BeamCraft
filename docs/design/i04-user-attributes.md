# I04: user attributes

Date: 2026-10-30. Status: **experimental slice.** Contract: upstream #531 /
axisrow/BeamCraft#316 (M3), depends on F04 and U02 (both delivered).

Result scope of this card: defined attributes (text, number, integer, boolean, choice, with
units) on any element, attribute sets. Acceptance: stored in USD; an unknown attribute is shown,
never dropped. The property editor is UI-side and not part of this card.

## Storage (USD)

**Definitions** live in a dedicated mutable schema scope, one prim per attribute:

```
/World/Definitions/Attributes            (Scope)
/World/Definitions/Attributes/<Name>     (Scope, one per definition)
    bim:attr:type    token    text | number | integer | boolean | choice
    bim:attr:unit    string   optional, present exactly when a unit was defined
    bim:attr:options string[] choice only, non-empty, unique
    bim:attr:set     string   optional, the attribute set this definition belongs to
```

Definitions are schema metadata, not elements: they carry no UUIDv7, no `bim:elementKind`, no
placement and do not appear in element listings. They are mutable (define/undefine), unlike the
digest-pinned definitions beside them.

**Values** are custom attributes `bim:attr:<Name>` on any element prim (plate, beam, column,
assembly, connection, level, grid, reference wrapper). The USD type follows the definition:
`string`, `double`, `int`, `bool`, `string` (choice). The value is stored in the unit named by
the definition; nothing converts it. Millimetre-based units are labels only, as everywhere else.

**Unknown attributes** are `bim:attr:*` properties with no definition: values left by a foreign
USD tool, or values that outlived an undefine. Undefining a definition never touches stored
values. All edits are targeted property writes (`attr()`), so no edit path rewrites a prim's
property set: an unknown attribute survives every later edit of its element. The reader
(`attribute-get`) reports unknown attributes next to defined ones, marked `defined: false`, with
their stored USD type and raw value. Foreign content stays inert, as everywhere else.

## Names and bounds (`model/src/limits.rs`)

A name must be a valid USD identifier: `[A-Za-z_][A-Za-z0-9_]*`, 1..=64 bytes — the property
name and the definition prim name are then the attribute name verbatim, lossless in USDA.
Text values and option strings are 1..=256 bytes without control characters (the label rule);
a unit is 1..=16 bytes; a set name 1..=64 without control characters; choice has 1..=64 options, unique; a number is finite
with |v| <= 1e9; one element carries at most 256 values; a model defines at most 1024 attributes.

## Commands and layers

Model (`model/src/attribute.rs`): `AttrType`, `AttrValue`, `AttrDef`, and the commands
`DefineAttribute`, `UndefineAttribute`, `SetAttribute`, `ClearAttribute`, each with admission
and a content fingerprint. Setting an undefined attribute is refused (`NOT_FOUND`);
clearing is refused only when no value is present (`INVALID_ARGUMENT`). Values outlive their
definition.

Engine (`engine/src/attribute.rs`): pure decisions over the decoded schema and values —
duplicate define (`DUPLICATE_ID`), missing definition or element (`NOT_FOUND`), type/option/
bound checks, no-value clear. Footprints: a schema write targets `Target::AttrSchema`
(reserved identity `bim:attr-schema`, lifecycle group); a value write targets
`Target::Element(id)` with the new `Group::Attributes`, so an attribute edit conflicts with
another attribute edit of the same element and with nothing else.

Layer engine: `Change::AttrDefine`, `Change::AttrUndefine`, `Change::AttrSet` (targeted
`attr()` writes; `None` removes the property). The candidate validator walks the schema scope
(owned, parse-or-fail) and re-checks value type consistency; foreign or unparseable schema
content stays inert. No element machinery changes: definitions are not elements.

CLI (`attribute-define`, `attribute-undefine`, `attribute-set`, `attribute-clear`,
`attribute-get`, `attributes`): the four mutations publish revisions through the same
Author/prepare/publish path as every edit (U02, U06/U07), with head-state replay semantics as
in `update-element` (a retry that finds its own state replays; a contradicting reuse is
`OP_ID_REUSE`; an unseen op that changes nothing is `INVALID_ARGUMENT`). `attribute-get` lists
an element's values, known and unknown; `attributes` lists definitions and their sets.

Attribute sets here are grouping on a definition (`--set`), stored and listed; applying a set
is what a property editor would do and stays with the UI card. `copy-element` does not carry
values yet (values are not in the typed copy path); the source keeps them.

## Evidence

Engine unit tests (admission, bounds, unknown attributes) and a real-binary CLI suite:
define/set/get round trip, value survival across `update-element` and across undefine (the
unknown-attribute acceptance), USD layer bytes read back through the generic reader, and the
adversarial refusals above.
