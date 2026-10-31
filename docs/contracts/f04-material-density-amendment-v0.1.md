# F04 amendment: material density may be zero

Status: **accepted amendment (2026-10-30), owner decision.**
F04 0.1.0 and F06 0.1.0 stay frozen; accepted amendments are versioned alongside them, and the two
rules they replace carry a dated pointer to this text.

## Rule

A material's density, when authored, is **at least zero** and at most 1,000,000 kg/m3. Absence is
still `unknown`.

| Density | Admission | Mass |
|---------|-----------|------|
| absent | admitted | `unknown`, never zero or defaulted |
| negative | `INVALID_ARGUMENT` (`GEO_NOT_POSITIVE` in the kernel) | none |
| exactly `0` | admitted | `current`, exactly `0` kg |
| positive | admitted | `volume * density` as before |

A positive density whose product with the volume underflows to zero is still
`GEO_NUMERIC_UNCERTAIN`: only an explicit zero density gives a zero mass.

## Why

Auxiliary, non-steel parts need a material whose mass is a known zero, so a bill of materials can
tell "carries no weight" from "mass not known". The built-in model material `Steel_Undefined`
has density `0`.

## What it changes

* [F04](f04-model-and-commands-v0.1.md) section 3 limits row "Material density, when authored":
  "greater than zero" becomes "at least zero".
* [F06](f06-geometry-contract-v0.1.md) validation table (density) and the `GEO_NOT_POSITIVE`
  meaning: "not greater than zero" becomes "negative" for a density. The statement that an
  unknown density keeps mass unknown and is never taken as zero stays: it is about an absent
  density, not an explicit zero.
* Nothing in an existing store changes: a stored density was always positive.
