# I04 typed user attributes

Contributor-original Rust and synthetic fixtures, MPL-2.0.

`define-attribute` creates an immutable UUID-backed definition with a unique case-sensitive UTF-8
name, type, optional numeric unit label, choice list and named set memberships. Supported types
are text, number, exact int64, boolean and choice. Definitions are bodyless metadata elements at
`/World/Attributes/ATD_<uuid>`; they never enter the engineering element list, BOM, numbering or
IFC proxy list. They cannot be changed or deleted through this CLI surface. The model schema pin
changes; older pre-alpha stores are refused explicitly rather than migrated.

Per-element values are native USD scalars named `user:a_<definition-uuid-without-hyphens>`.
The `a_` prefix makes the UUID a valid USD identifier. Values are local to each element, including
P11 instances; they do not share the part's shape/material definition. `set-attributes --id UUID
--values FILE.json` takes an object keyed by definition UUID or exact name. A plain numeric
value is in the definition's unit; `{ "value": 12, "unit": "mm" }` checks an exact unit label.
There is no implicit conversion, truncation, missing-to-zero or false-to-zero coercion.
`remove-attributes --attributes UUID_OR_NAME,...` removes only the explicitly named properties.

`list-attributes` supports set membership and bounded offset/limit pagination (default 16, max
64). `inspect-attributes` shows native property names/types, UUID/definition when known, exact
values and blocked/unset state. Unknown user properties remain visible and survive ordinary
geometry, metadata and known-attribute edits. A copy carries admitted known scalar values;
unknown, opaque, blocked or unresolved user data refuses copying instead of silently disappearing.
This applies to ordinary copies, linked copies, mirrors and arrays.

There are at most 256 definitions, 64 properties per element/patch, 128 choices and 16 set labels
per definition. Text values are at most 256 bytes; numeric doubles follow the existing static USD
reader's absolute 1e9 scalar ceiling; int64 values remain exact. Existing layer/string/footprint
budgets also apply. An unsupported native type for a known definition is refused; absent or
blocked values do not acquire a default.

Engine admission derives coarse ShapeInputs metadata dependencies for properties and reads each
used definition's lifecycle/content. User properties are excluded from engineering body/quantity
signatures, so this conservative conflict grouping does not rebuild or invalidate geometry.
The layer adapter captures bounded dynamic property names and preserves unrelated opinions.
Rebase classifies the owned user/definition namespace with the same conservative group.
One accepted patch is one revision and undo unit. Immutable request evidence provides replay and
OP_ID_REUSE after subsequent changes without applying an old patch twice.

Real CLI tests cover all scalar types, exact int64 max, units, choices, refusal atomicity, replay
and reuse, copied local instance values, unknown preservation, safe copy refusal, undo and redo.
Engine admission tests and the previous T03 group/mirror/array tests run alongside them. Native
property editors and schedules are separate tasks; this slice does not claim their acceptance.
