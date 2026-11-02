# Changing the profile of a placed member

Date: 2026-10-30. Status: experimental slice; no gate acceptance is claimed.

## Problem

A member's profile could only be chosen when it was created. `update-element` had no profile
field and the properties panel showed the profile as text, so a wrong section meant deleting and
placing the member again. The owner requires the window to change it.

## What changed

* `ElementPatch.profile` (model): the new pinned profile definition of a member. It takes part in
  the "patch changes nothing" check and in the content fingerprint (appended only when present,
  so earlier fingerprints are unchanged).
* Engine `update`: a profile patch is a shape change (`UpdateEffect.profile` and `shape`). The
  member is repinned, its body is rebuilt from the new section and the mass is recomputed. A
  plate has no profile: `UNSUPPORTED_PROPERTY`. `Footprint::for_profile_change` reads the new
  profile definition and writes the element's relationship group.
* CLI: `update-element --profile NAME | --profile-digest SHA256`. A name is resolved as for
  `create-member`: among the model's definitions, then in the project's environment, where it is
  defined in the model first (see [the blank environment](blank-environment.md)). The layer
  engine's existing `Change::Profile` rewrites the `bim:profile` relationship of the canonical
  part, so parts that share one definition follow the change; the reply reports
  `effect.profile`.
* Window: the properties panel shows a Profile list for a beam or column (the model's profiles
  and the environment's); applying it runs `update-element` with the digest, or with `--profile`
  for an entry that only the environment has.

## Not done

Creating a new profile in the window, and changing the section of several selected members at
once. A retried command with the same `op_id` is decided from the head state, as for the other
patches.
