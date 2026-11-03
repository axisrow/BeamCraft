// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the session commands.

pub(super) const HELP: &str = concat!(
    "Session: session --store ROOT (one beamcraft process holding one opened store; one ",
    "command invocation per stdin line, exactly the option grammar of the one-process CLI ",
    "with its refusals and JSON shapes, and one JSON object per stdout line; the agent ",
    "starts the process and owns its pipes, so there is no socket, no listener and nothing ",
    "enabled by default; the mode ends at EOF on stdin; every line must name the ",
    "session's own --store; the export commands and a nested session are refused because ",
    "they would write past the line protocol; a store without a revision creates its ",
    "first revision with an unbracketed command — only batch-open needs a head; read-only). ",
    "Batch: batch-open, batch-commit [--op-id UUID] [--revision-id UUID] ",
    "[--timeout-ms MS] [--cancel-after-ms MS], batch-abort (between batch-open and ",
    "batch-commit the modelling commands accumulate as an in-memory draft and touch no ",
    "disk; batch-commit publishes the whole batch as one revision with one operation id, ",
    "checked against the base pinned at batch-open, so a head that moved mid-batch fails ",
    "the batch instead of rebasing it; a refusal abandons the batch whole and the session ",
    "stays alive; nothing partial is ever published; each carried command stays replayable ",
    "by its own operation id through the batch revision's evidence; only modelling ",
    "commands may run between the brackets, and reads answer from the committed store, ",
    "not the draft; a session that ends with an open batch abandons it and says so on its ",
    "last line)",
);
