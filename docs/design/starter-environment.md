# Starter environment and seeded new models

Date: 2026-10-30. Status: experimental slice; no gate acceptance is claimed.

## Problem

A new model had no profile and no material definitions. The native window lists the definitions
of the model, so the beam and column tools offered an empty list and nothing could be placed.
The `blank` environment is empty on purpose and the environment chosen on the start window was
only a saved setting.

## What changed

* `environments/starter/` is a built-in A07.3 environment with nineteen generic profiles
  (I, H, channel, angle, RHS, SHS, CHS, bar and one cold-formed C). Every value is invented and
  rounded; none follows a standard or catalogue (`AGENTS.md` 1.2). The start window prefers it
  over `blank` when no environment is saved.
* `create-project --environment ID` looks the environment up (built-in, then the user's), reads
  and admits it, then copies it into the project folder (`environment/`) and publishes the two
  default materials. The project therefore carries its environment: closing and reopening it, or
  opening it on another machine, needs no setting and no option. The reply gains
  `seeded {environment, materials}`. An unknown or unreadable environment is refused and the
  folder this call made is removed again. Without `--environment` nothing changes.
* A profile is not copied into the model. `--profile NAME` of `create-member`, `create-polybeam`
  and `create-curved-beam` first looks among the model's definitions; when the name is not there it
  looks in the project's environment and defines that entry in the model (content-addressed, so
  idempotent) before the member is created. This is two revisions, not one: a failed member
  leaves the harmless definition. `definitions` lists the environment entries the model does not
  define yet as `environment_profiles`.
* The window passes the chosen environment to `create-project` and lists those entries in the
  profile picker; a pick that is only in the environment is created with `--profile NAME`.
* Default materials: `Steel_Undefined` (density 0) and `Steel` (7850 kg/m3, no grade).

## Zero density

`Steel_Undefined` is the material of auxiliary, non-steel parts. Its density is exactly zero, so
its mass is a known `current` 0 kg, not `unknown` (an absent density stays `unknown`). A negative
density is still refused (`INVALID_ARGUMENT`; `GEO_NOT_POSITIVE` in the kernel). A positive
density whose product underflows to zero is still `GEO_NUMERIC_UNCERTAIN`: only an explicit zero
gives a zero mass. The F04 admission "density greater than zero" is amended to "at least zero" in
[the density amendment](../contracts/f04-material-density-amendment-v0.1.md).

## Not done

Real regional catalogues (the policy of `AGENTS.md` 1.2 applies), typed designations such as
`I 300*150*7*10`, user-defined parametric templates, and profile definition from the window.
Libraries in the project folder beyond the copied environment, and the user library, are not
resolved yet.
