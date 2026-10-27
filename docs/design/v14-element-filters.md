# V14 shared typed element filters

Contributor-original Rust and synthetic fixtures, MPL-2.0.

The engine owns one admitted immutable Dataset and rule evaluator for direct queries, saved-view
visibility and later representation rules. Rules are AND (`all`) / OR (`any`) trees or typed
comparisons on `type`, `name`, `class` or an explicit `attribute` selector. Attribute names resolve
once to persistent UUIDs. Unknown fields/attributes, incompatible literal types and operators
refuse before matching; missing/blocked values never match a comparison, including `ne`.
Empty AND is true, empty OR false. Rules have at most 256 nodes and depth eight.

Text/choice supports exact equality/inequality, case-sensitive substring and prefix. Numeric
fields support equality and ordered comparisons; booleans support equality/inequality. Integers
remain exact. There is no unit conversion: numeric user literals may carry `{value,unit}` and
must match the definition. Saved rules normalize selectors to UUIDs and always retain numeric
unit labels, so a future incompatible definition cannot silently reinterpret their literals.

`query-elements --store ROOT (--rule FILE.json | --filter UUID)` returns sorted UUIDs with a
model-manifest pin and bounded pagination (`--limit`, `--after`). `--expected-model` refuses a
page from a changed model. Registry definition records are not model objects; inactive targets
are excluded. Physical snapshot visibility additionally intersects the resulting IDs with the
engineering snapshot and the V15 conservative clip mask.

`save-filter`, `list-filters`, `inspect-filter` and `delete-filter` use the same W01 personal
store and expected-file-digest CAS. A filter is a named UUID and canonical rule JSON.
`update-view --filter UUID|none` binds/unbinds that exact rule to a camera. A referenced filter
cannot be deleted; deleting the view removes its binding. Stored rule inspection retains its
JSON; unknown definitions are diagnosed when the rule is compiled, never replaced with show-all.
No filter, binding or display query writes a model revision or changes numbering.

The model inspection now exposes the already-decoded pinned user-property context, avoiding a
second independent model read during view projection. Dynamic property type checks reuse admitted
definitions without repeatedly validating their entire choice/set registry per element.

CLI tests cover API/query/view ID parity, nested groups, part classes, case-sensitive names,
missing choice values, private CRUD/dependency refusal, pagination/model pins, unknown fields,
invalid operators, localized attribute names and exact numeric units. Engine tests check missing
comparison semantics and unknown-definition admission. Native editors remain outside this scope.
