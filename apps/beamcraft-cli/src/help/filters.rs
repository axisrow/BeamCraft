// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the element filter command family (V14).

pub(super) const HELP: &str = concat!(
    "Element filters (V14): query-elements --store ROOT --rule FILE.json [--limit 1..1024] ",
    "[--after UUID] [--expected-model DIGEST|none]; the reply lists matching element UUIDs in ",
    "canonical order with a `next` cursor and pins the head manifest. A rule is an ",
    "{\"all\":[...]} or {\"any\":[...]} group or one comparison {\"field\":type|name|class|attribute, ",
    "\"attribute\":NAME, \"op\":eq|ne|lt|le|gt|ge|contains|starts-with, \"value\":...}; a numeric ",
    "literal may be {\"value\":N,\"unit\":\"mm\"} and must match its definition's unit. Unknown ",
    "fields and attributes and incompatible operators refuse before matching; a missing value ",
    "never satisfies a comparison, including ne. type, name and class are the standard fields; ",
    "only plates, beams and columns carry a class. Saved sets and view bindings land with the ",
    "views layer (V12, W01)"
);
