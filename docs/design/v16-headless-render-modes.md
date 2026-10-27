# V16 headless display modes

`export-view --store ROOT --out - --render-mode MODE [--select UUID,...]`
returns BCVXFRM3 presentation JSON version 2 and the unchanged engineering bundle.
Modes: wireframe (edges only), transparent (lit, transparency 0.65), grey (unlit
uniform grey), shaded (lit), selected-only (lit, explicit selection intersection).
An omitted selection is empty. Repeated identities collapse; unknown identities
fail through the existing snapshot-bound Selection API. Filters and clipping apply
before selected-only. With no saved view, `view` is null and consumers retain their
camera. No display options preserves the legacy raw/V1/V2 transport.

`--select-file FILE` accepts a JSON UUID array (4 MiB, 65,536 entries), mutually
exclusive with `--select`. Presentation metadata has an explicit 32 MiB bound;
file-output summaries report identity counts instead of large lists. Frame sections
remain u32-length-prefixed, and snapshot tokens and engineering geometry are unchanged.

Engine CPU test: five queries over 2,000 synthetic elements took 7.8 ms locally
(debug, informational, no timing assertion). This is not GPU frame-time acceptance.
CLI tests exercise five modes, empty selection, selection files, invalid input and
byte-identical bundle output. Windows, keyboard shortcuts and renderer pipelines
remain UI work.
