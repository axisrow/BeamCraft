# V14 element filters

Contributor-original BeamCraft code and documentation, MPL-2.0.

One bounded evaluator in `beamcraft_engine::filter` compiles a rule against an immutable
`Dataset` before any element is evaluated. A rule is `all`/`any` groups or one typed
comparison over the standard fields `type`, `name`, `class` (P15) or a user attribute by
definition name (I04). Operators are closed (`eq`, `ne`, `lt`, `le`, `gt`, `ge`, `contains`,
`starts-with`); text fields and choice attributes take equality, substring and prefix;
booleans take equality only; unknown fields, unknown attributes and incompatible operator or
literal types refuse before matching. A missing value never satisfies a comparison, not even
`ne`: there is no implicit missing-to-zero or name/class fallback. Rules are bounded at 256
nodes and depth eight; datasets at `MAX_ELEMENTS` targets.

`query-elements --store ROOT --rule FILE.json [--limit 1..1024] [--after UUID]
[--expected-model DIGEST]` runs the same evaluator over the verified head through the CLI and
replies with matching UUIDs in canonical order, a `next` cursor and the head manifest digest
as a pin. A numeric literal may be `{"value":N,"unit":"mm"}` and must match its definition's
unit exactly. A query never changes the model.

Only active elements are targets; registry definitions are not objects. A record carries a
value only when its stored type matches its definition, so a leftover presence from an
earlier definition is reported as unknown, never matched. Only plates, beams and columns
carry a part class; other categories have none and never match a class rule.

Deliberately not in this slice, so the seams stay honest: saved personal sets, view-filter
bindings and the export-view visible mask need the views layer (V12, W01), which is not
re-delivered yet; the evaluator and the rule JSON are exactly the pieces those tasks
evaluate. The UI filter editor rides the same commands (V10/V12). An earlier delivery of this
task was lost with the pre-reset master line; the evaluator is its reviewed design ported to
the name-keyed attribute schema.
