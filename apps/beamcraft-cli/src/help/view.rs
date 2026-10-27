// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the view command family.

pub(super) const COMMANDS: &[&str] = &[
    "snapshot-view",
    "inspect-view-changes",
    "save-view",
    "list-views",
    "inspect-view",
    "update-view",
    "delete-view",
    "restore-view",
    "create-view",
    "set-view-clipping",
    "clear-view-clipping",
    "query-elements",
    "save-filter",
    "list-filters",
    "inspect-filter",
    "delete-filter",
];

pub(super) const HELP: &str = concat!(
    "Filters (V14): query-elements --store ROOT (--rule FILE.json | --filter UUID) [--limit 1..1024] [--after UUID] [--expected-model DIGEST]; save-filter --workspace-dir DIR --name TEXT --rule FILE.json [--id UUID] [--expected-views DIGEST|none]; list-filters --workspace-dir DIR; inspect-filter|delete-filter --workspace-dir DIR --id UUID. Rules use all/any arrays or {field:type|name|class|attribute, attribute:NAME_OR_UUID, op:eq|ne|lt|le|gt|ge|contains|starts-with, value:...}. Unknown fields/attributes and incompatible types refuse; missing values never match comparisons. update-view --filter UUID|none binds the same evaluator to export-view.\n",
    "View clipping (V15): set-view-clipping --workspace-dir DIR --id UUID [--planes PX,PY,PZ,NX,NY,NZ;...|none] [--work-area MINX,MINY,MINZ,MAXX,MAXY,MAXZ|none] [--depth-enabled true|false]; clear-view-clipping --workspace-dir DIR --id UUID disables all planes, depth and work area. Both accept --expected-views DIGEST|none. Positive half-spaces remain visible; engineering geometry and measurements are unchanged.\n",
    "Plane views (V09): create-view --workspace-dir DIR --kind 3d|plane|two-points|level|grid-line --name TEXT [--id UUID] [--plane ORIGIN,XDIR,YDIR] [--start X,Y,Z --end X,Y,Z] [--level UUID] [--grid ID_OR_NAME --axis AXIS_UUID] [--side positive|negative] [--depth MM] [--width MM] [--aspect F] [--comment TEXT] [--work-area MINX,MINY,MINZ,MAXX,MAXY,MAXZ] [--expected-views DIGEST|none]. Persistent grid/axis and level identities re-resolve on restore; absent anchors return status unresolved. Two points define a vertical plane; view state never edits the model.\n",
    "Personal views (W01): save-view --workspace-dir DIR --name TEXT [--id UUID] [--position X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--projection orthographic|perspective] [--width MM] [--aspect F] [--fov RADIANS] [--plane ORIGIN,XDIR,YDIR] [--comment TEXT]; list-views --workspace-dir DIR; inspect-view|restore-view|delete-view --workspace-dir DIR --id UUID; update-view --workspace-dir DIR --id UUID [same camera fields] [--depth MM --side positive|negative] [--work-area MINX,MINY,MINZ,MAXX,MAXY,MAXZ|none]. All accept --expected-views DIGEST|none for optimistic concurrency. Views never change the model revision.\n",
    "View (synthetic kernel plate scene, no store): snapshot-view --plates 1..500 [--hide ",
    "KIND,...] [--representation full|simplified] [--stand-in-simplified true|false] ",
    "[--select ID,...] [--revision SHA256] [--generation U64]; inspect-view-changes --plates ",
    "1..500 --edit move-first|resize-first|add-one|remove-last,... [--since TOKEN] ",
    "[--stand-in-simplified true|false]",
);
