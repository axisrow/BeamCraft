# E01: own IFC export contract, version 0.1.0

Status: **draft 0.1.0.** Implemented by `export-ifc` and `ifc-guid`; it is frozen once the checks of
section 19 pass.

This contract fixes **what BeamCraft writes to an IFC file, how an element becomes an IFC entity,
how its 22-character GlobalId is derived, what is lost, and how the writer is built and driven.**
The examples are acceptance descriptions, not APIs or saved files.

## 1. Status, scope and provenance

### 1.1 Relation to other contracts

- [F04](f04-model-and-commands-v0.1.md): identities (section 2), units and caps
  (section 3), quantity statuses (section 5) and the 23 error codes (section 8). E01 changes
  nothing there and adds no error code.
- [F06](f06-geometry-contract-v0.1.md) (frozen 0.1.0): the canonical box mesh and its face order
  (section 6), the quantity definitions (section 8), the key and signature rules (section 7) and
  the `GEO_` code table, which is the model for the `IFC_` table of section 13.
- [F10](f10-domain-vocabulary-v0.1.md) (draft): the closed element registry (`plate`, `beam`, `column`
  implemented, ten kinds reserved), unknown-content preservation, the validation envelope and
  its ordering (sections 6.3 and 6.4).
- [Module boundaries (F08)](f08-module-boundaries-v0.1.md): where the writer and the adapter live
  (section 15.1).
- The stored model: committed members and plates (with their holes), the stored quantity records
  and their freshness, and the declared section values of profile definitions. Only what is
  published is exported (section 8).

### 1.2 What the document decides

1. The IFC schema and file format, and the supported entity subset (section 2).
2. The deterministic GlobalId, its exact encoding, its test vectors, its failure behaviour and
   where it is published (section 3).
3. Which committed data is read, which elements are eligible and in what order (section 4).
4. Spatial structure, placement, mesh representation, material, quantities, property sets and
   units (sections 5 to 10), each row pointing to a public buildingSMART specification section
   (section 18).
5. Determinism rules (section 11), the explicit list of losses (section 12), the diagnostic codes
   (section 13), size bounds (section 14) and the headless surface and writer design (section 15).

### 1.3 Excluded

- **IFC import and conversion of foreign models.** Reading IFC, and turning IFC into BeamCraft
  elements or read-only references, needs a separate scope decision. This contract
  defines no reading API (`IFCX-SCOPE-003`). A test harness may parse a written file to check it;
  that is not a product feature.
- BOM and reports, and drawings.
- Any IFC feature outside the subset of section 2, including model view definition conformance,
  classification, structural analysis, connections, assemblies, bolts, welds and reinforcement.
- Remote, collaborative or networked export. Export is a local, read-only operation.

### 1.4 Sources

Behaviour comes only from the public buildingSMART IFC specification (entity definitions,
property and quantity sets, the GlobalId type and the STEP physical file format); section 18 lists
every page used. No code, schema listing, example file or algorithm is taken from any IFC library
or commercial product. The GlobalId test vectors of section 3.2 are BeamCraft's own.

## 2. IFC version and supported subset

### 2.1 Schema and file format

| Item | Value | Source |
|------|-------|--------|
| Schema identifier in `FILE_SCHEMA` | `IFC4` (the IFC4 ADD2 TC1 edition) | S01 |
| File format | Clear-text STEP physical file (`.ifc`), 7-bit ASCII, LF line ends | S02. `ifcXML`, `ifcJSON` and `ifcZIP` are not produced (`IFCX-VER-002`) |
| Model view definition | **None claimed** (`IFCX-VER-003`) | - |
| Contract version in requests | Exactly `0.1.0` (`IFCX-VER-001`) | F04 section 6 pattern |

IFC4 is read by more tools than IFC 4.3 and has every entity the subset needs. Its consequences,
all stated where they apply: the plate base quantity set has no weight quantities (section 8.2),
and the enumeration values of the three product types are those of IFC4 (section 2.2).

**Model view.** The tessellated representation of section 6 is the form that the IFC4 Reference
View is built around, so that view is a candidate. The contract does not claim it: the Reference
View's complete requirements were not available to the author, and a header that names a view
asserts conformance to all of it. Whether the written file satisfies the published Reference View
definition is an acceptance check (section 19); only then may `FILE_DESCRIPTION` name it, in a
later version.

### 2.2 Entity subset

Every entity below is written by a fixed template (section 15.2); the writer has no generic IFC
object model. The spec column cites section 18.

| ID | Entity | Used for | Spec |
|----|--------|----------|------|
| E-01 | `IfcProject` | One per file; units and representation context | S10 |
| E-02 | `IfcSite`, `IfcBuilding` | One synthetic site and one synthetic building above the stored levels | S11, S12 |
| E-03 | `IfcBuildingStorey` | One per stored Level | S13 |
| E-04 | `IfcRelAggregates` | Project to site, site to building, building to storeys | S14 |
| E-05 | `IfcRelContainedInSpatialStructure` | Elements in their storey, or in the building when they have no level | S15 |
| E-06 | `IfcBeam`, `IfcColumn`, `IfcPlate` | One per element of kind `beam`, `column`, `plate` | S16, S17, S18 |
| E-07 | `IfcBuildingElementProxy` | One per element of any other kind | S19 |
| E-08 | `IfcLocalPlacement`, `IfcAxis2Placement3D`, `IfcCartesianPoint`, `IfcDirection` | Placement | S20 to S22, S23 |
| E-09 | `IfcProductDefinitionShape`, `IfcShapeRepresentation` | The body representation | S24, S25 |
| E-10 | `IfcGeometricRepresentationContext`, `IfcGeometricRepresentationSubContext` | One model context and one Body sub-context | S26, S27 |
| E-11 | `IfcTriangulatedFaceSet`, `IfcCartesianPointList3D` | The only geometry item | S28, S29 |
| E-12 | `IfcMaterial`, `IfcRelAssociatesMaterial`, `IfcMaterialProperties` | Pinned material, its density, its digest | S30, S31, S32 |
| E-13 | `IfcPropertySet`, `IfcPropertySingleValue`, `IfcRelDefinesByProperties` | `BeamCraft_Element`, `BeamCraft_Declared` | S33, S34, S35 |
| E-14 | `IfcElementQuantity`, `IfcQuantityVolume`, `IfcQuantityWeight` | The base quantity sets | S36 to S38, S42 to S44 |
| E-15 | `IfcUnitAssignment`, `IfcSIUnit`, `IfcDerivedUnit`, `IfcDerivedUnitElement` | Units | S39, S40, S41 |
| E-16 | Types `IfcGloballyUniqueId`, `IfcLabel`, `IfcIdentifier`, `IfcText`, `IfcMassMeasure` | Attribute and property value types | S03 to S07 |

Predefined types (IFC4 enumerations, S45 to S48): `IfcBeam` gets `.BEAM.`, `IfcColumn` gets
`.COLUMN.`, `IfcPlate` and `IfcBuildingElementProxy` get `.NOTDEFINED.`. (The IFC4 plate
enumeration is curtain panel, sheet, user defined and not defined; BeamCraft has no purpose
attribute, so it claims none.) `IfcRoot.OwnerHistory` is written as unset (`$`); whether the
validation tool requires it is checked at acceptance.

### 2.3 Not in the subset

`IfcExtrudedAreaSolid`, profile definitions, boolean results, `IfcOpeningElement` and
`IfcRelVoidsElement`, `IfcPolygonalFaceSet`, brep items, `IfcBeamType`, `IfcColumnType`,
`IfcPlateType`, `IfcMaterialProfileSet` and usage entities, styles and colours, grids,
classification references, `IfcOwnerHistory`, groups, assemblies and connections, annotations and
any analysis entity.

## 3. Identity and the GlobalId

### 3.1 The encoding E

`IfcGloballyUniqueId` is a 22-character string (S03). E(u) encodes the 128 bits of a UUID as follows.

1. **Input.** The 16 bytes of the UUID in the order of the canonical text, byte 0 first (RFC 9562
   section 4 network order, which is `Uuid7::as_bytes`). The bytes are read from the hex digits of
   the `8-4-4-4-12` text from left to right; no field is swapped or reversed.
2. **Number.** Read the 16 bytes as one unsigned 128-bit big-endian integer `N`.
3. **Digits.** Write `N` in base 64 as 22 digits, most significant first: the first digit is
   `N >> 126`, which is 0 to 3 (2 bits); each of the other 21 digits is the next 6 bits. In byte
   terms, byte 0 supplies the first two digits (its top 2 bits, then its low 6 bits) and every
   following group of 3 bytes supplies 4 digits. This is the layout the public type definition
   describes, and the first digit is always 0, 1, 2 or 3.
4. **Alphabet.** Digit value 0 to 63 maps to `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$`
   (S03): digits `0`-`9` are values 0 to 9, `A`-`Z` 10 to 35, `a`-`z` 36 to 61, `_` is 62 and `$` is 63.

E is a bijection between 128-bit values and the 22-character strings whose first character is in
`0123`. It is not RFC 4648 base64: it has no padding, uses `_$` instead of `+/` and starts with a
2-bit digit.

The ASCII order of GlobalIds is not the numeric order of the UUIDs, because `_` (0x5F) sorts before
`a` (0x61) but has the larger digit value. **Every list in the file and the report is therefore
ordered by ElementId (or LevelId), never by GlobalId** (`IFCX-DET-001`, case `E01-POS-11`).

### 3.2 Test vectors (hand-derived)

The vectors were derived by hand from the bit groups and then confirmed with a throwaway script
that also checked decode(encode(x)) = x. They are the contract; an implementation that disagrees is
wrong. Vectors 1 to 5 are encoder-only (their version bits are not 7, so they are not valid
ElementIds); 6 to 11 are valid UUIDv7 text.

| # | Vector | UUID text | GlobalId | Note |
|---|--------|-----------|----------|------|
| 1 | V-ENC-01 | `00000000-0000-0000-0000-000000000000` | `0000000000000000000000` | encoder-only (version nibble 0) |
| 2 | V-ENC-02 | `ffffffff-ffff-ffff-ffff-ffffffffffff` | `3$$$$$$$$$$$$$$$$$$$$$` | encoder-only (version nibble f) |
| 3 | V-ENC-03 | `80000000-0000-0000-0000-000000000000` | `2000000000000000000000` | encoder-only |
| 4 | V-ENC-04 | `00000000-0000-0000-0000-00000000003f` | `000000000000000000000$` | encoder-only |
| 5 | V-ENC-05 | `00000000-0000-0000-0000-000000000040` | `0000000000000000000010` | encoder-only |
| 6 | V-ENC-06 | `019a7bf0-0000-7000-8000-000000200001` | `01cdlm001m080000008001` | valid ElementId |
| 7 | V-ENC-07 | `019a7bf0-0000-7000-8000-000000000005` | `01cdlm001m080000000005` | valid ElementId |
| 8 | V-ENC-08 | `01234567-89ab-7def-bfed-cba987654321` | `018qLdYQjzxx$jowc7PKCX` | valid ElementId |
| 9 | V-ENC-09 | `0197c3a8-5e21-7b4f-9a06-3d8c1e72f4b9` | `01byEeNY5xJve6FOmUSlIv` | valid ElementId |
| 10 | V-ENC-10 | `019a7bf0-0000-7000-8000-000000000024` | `01cdlm001m08000000000a` | valid ElementId, last byte 36 |
| 11 | V-ENC-11 | `019a7bf0-0000-7000-8000-00000000003e` | `01cdlm001m08000000000_` | valid ElementId, last byte 62 |

Worked example, vector 6. The bytes are `01 9a 7b f0 00 00 70 00 80 00 00 00 00 20 00 01`. The first
digit is the top two bits of `01`, which is 0. The other 126 bits in groups of six have the values
1, 38, 39, 47, 48, 0, 0, 1, 48, 0, 8, 0, 0, 0, 0, 0, 0, 8, 0, 0, 1, which are the characters
`1 c d l m 0 0 1 m 0 8 0 0 0 0 0 0 8 0 0 1`, giving `01cdlm001m080000008001`.

The first two characters of the GlobalId of an ElementId whose millisecond timestamp falls between
2004 and 2039 are `01`, because they come from the top byte of the 48-bit millisecond timestamp.
GlobalIds therefore share a prefix. This is an observation, not a rule; the timestamp is a creation
hint and never an authority (F04 section 2).

### 3.3 Input that is not an ElementId

`IFCX-GUID-002`. The GlobalId function takes the text of `bim:id` and succeeds only when
`beamcraft_model::id::Uuid7::parse` succeeds: 36 characters, hyphens at positions 8, 13, 18 and 23,
**lowercase** hexadecimal, version nibble 7 and RFC variant. Anything else gives `INVALID_ID`, no
GlobalId, and no repair. In particular the function does not lower-case upper-case text, remove
braces or hyphens, accept a version 4 or other-version UUID, pad or truncate, or fall back to a
hash of the text. A fallback would mint an identity that BeamCraft's own identity rules (F04 section
2, F10 `DV-BASE-001`) do not recognise, and two different malformed texts could silently collide.

An export that meets a stored element with such an id is refused with `INVALID_ID`: since no element
is skipped (section 4.2), a revision that holds an invalid identity is not a valid revision. The
test vectors 1 to 5, which are not valid ElementIds, exercise the 128-bit function E of 3.1 only;
the ElementId check is a separate step in front of it, so the vectors can be checked without
weakening the rule.

### 3.4 Decoding

`IFCX-GUID-003`. D(g) accepts exactly 22 characters, each in the alphabet of 3.1, the first being
`0`, `1`, `2` or `3`. Any other text (length 21 or 23, a character such as `+`, `/`, `=` or a
non-ASCII character, a first character `4` or higher) is `INVALID_ARGUMENT`, because the value
would not fit in 128 bits or is not an IfcGloballyUniqueId. D(E(u)) = u for every 128-bit value.
Decoding is a helper for external matching (section 15.3); it does not turn the result into an
ElementId unless it also passes `Uuid7::from_bytes`.

### 3.5 Synthetic GlobalIds

Entities that are not elements but are `IfcRoot` subtypes (project, site, building, storeys,
relationships, property sets, quantity sets) need a GlobalId too (`IFCX-GUID-004`). They use this construction:

`G(role, keys) = E( first 16 bytes of SHA-256( "BCIFC-GUID" 0x00 role 0x00 key_1 ... key_k ) )`

with SHA-256 as in `beamcraft_model::digest` (FIPS 180-4), `role` the ASCII token below and each
key a fixed-width byte string, so the preimage is unambiguous. The ProjectId is the one **stored in
the project entry** (`IFCX-PRJ-001`, `IFCX-PRJ-002`).

| Entity | Role token | Keys |
|--------|-----------|------|
| `IfcProject` | `project` | ProjectId (16 bytes) |
| `IfcSite` | `site` | ProjectId |
| `IfcBuilding` | `building` | ProjectId |
| `IfcBuildingStorey` | `storey` | ProjectId, LevelId (16 bytes) |
| `IfcRelAggregates` project to site | `rel-project-site` | ProjectId |
| `IfcRelAggregates` site to building | `rel-site-building` | ProjectId |
| `IfcRelAggregates` building to storeys | `rel-building-storeys` | ProjectId |
| `IfcRelContainedInSpatialStructure` of a storey | `rel-contained-storey` | ProjectId, LevelId |
| `IfcRelContainedInSpatialStructure` of the building | `rel-contained-building` | ProjectId |
| `IfcRelAssociatesMaterial` | `rel-material` | ProjectId, material ContentDigest (32 bytes) |
| `IfcPropertySet` BeamCraft_Element | `pset-element` | ElementId (16 bytes) |
| `IfcPropertySet` BeamCraft_Declared | `pset-declared` | ElementId |
| `IfcElementQuantity` | `qto-element` | ElementId |
| `IfcRelDefinesByProperties` for BeamCraft_Element | `rel-pset-element` | ElementId |
| `IfcRelDefinesByProperties` for BeamCraft_Declared | `rel-pset-declared` | ElementId |
| `IfcRelDefinesByProperties` for the quantity set | `rel-qto-element` | ElementId |

Every row is written only when the entity is written (`IFCX-EMIT-001`, section 11.2): for example the
quantity-set rows exist only for an element with at least one quantity to write. These values are not
UUIDs and carry no version bits. Vectors `V-SYN-01` (role `site`, ProjectId
`019a7bf0-0000-7000-8000-000000000001`) is `2ghTuGpW1EXNtW6xLArrhQ`, and `V-SYN-02` (role `storey`,
the same ProjectId, LevelId `019a7bf0-0000-7000-8000-000000000101`) is `2886jimw3h7h75lUh8nRj7`; both were
derived with a throwaway script and must be reproduced with the in-repo SHA-256 before they are
trusted. `IfcMaterial`, `IfcMaterialProperties` and the geometry entities are not `IfcRoot` and
carry no GlobalId. A repeated GlobalId anywhere in one file refuses the export (`IFCX-GUID-006`);
this cannot happen for distinct valid ids and distinct roles, and the check exists so that it never
goes unnoticed.

### 3.6 Stability

`IFCX-GUID-007`. The element GlobalId is a pure function of the ElementId. It is the same after a
rename, an edit of any property, a move, a new revision, another export run, another platform and
another day. It ignores the element name, revision and every time. A copy has a new ElementId
(F04 `CopyElement`) and so a new GlobalId; the source's GlobalId never changes. The export report
lists both identifiers for every element, so a downstream program can map either way.

### 3.7 Publication

External programs match BeamCraft elements by GlobalId. The value is **computed at export and not
persisted** (`IFCX-GUID-008`):

1. The IFC file carries it as `IfcRoot.GlobalId` of every exported element.
2. The export report lists `element_id` and `ifc_global_id` for every element (section 15.4).
3. The CLI and the engine API expose the same function on demand (`ifc-guid`, and an
   `ifc_global_id` field in element inspection, section 15.4), so a program can obtain the value for
   an element without exporting.

No attribute is added to the authored or derived layer and the schema descriptor and its digest
do not change. Because the function is pure, every one of these values is reproducible from the
ElementId alone. A layer that already holds an attribute named `bim:ifcGlobalId` is carrying
unknown content: it is preserved, ignored and never trusted (F10 `DV-NS-002`).

## 4. Source data

### 4.1 What is read

`IFCX-SCOPE-002`. The export reads one verified committed revision through the host's verified reader:
by default the published head, optionally an explicit RevisionId. Layers and blobs are
verified; a revision that cannot be read completely is refused with `DEPENDENCY_UNAVAILABLE`. A
pinned snapshot of the revision is the only input (the same snapshot the F06 queries use), so all
data of one export comes from one revision. A personal workspace draft revision is exportable, if the
host exposes it, only through its own RevisionId. The export never reads a mutable working state.

`IFCX-SCOPE-004`, `IFCX-SURF-001`. The export changes nothing: no authored layer, derived layer,
revision, generation or journal is touched. Recording an export is not part of 0.1.0.

The project entry supplies the ProjectId (`IFCX-PRJ-001`) and the Levels (`IFCX-LVL-001`). A
revision whose project entry records no ProjectId is refused; no ProjectId is taken from an option
or invented.

### 4.2 Handling of every element

**No element is skipped** (`IFCX-SCOPE-006`) except a deactivated one (`IFCX-SCOPE-005`). Each
element goes through the checks below in this order; a failure of a request-level check refuses the
export, the others only degrade the element and are listed.

| Order | Check | Outcome |
|-------|-------|---------|
| 1 | `bim:isDeactivated` is `false` | otherwise not exported, listed as info |
| 2 | `bim:id` is a valid ElementId, unique in the revision | otherwise **request refused**: `INVALID_ID` or `DUPLICATE_ID` |
| 3 | Level, profile and material references resolve | otherwise **request refused**: `DANGLING_RELATIONSHIP` or `NOT_FOUND` |
| 4 | Kind is `plate`, `beam` or `column` | otherwise the element is an `IfcBuildingElementProxy`, listed (warning) |
| 5 | Placement is a rigid pose within the F04 caps | plate, beam, column: otherwise **request refused**, `INVALID_ARGUMENT`; proxy: exported without placement and representation, listed |
| 6 | A derived mesh exists, is valid and all faces are triangles | otherwise exported **without a representation**, listed (warning) |
| 7 | For plate, beam, column: the stored mesh equals the kernel's re-evaluation | otherwise **request refused**, `INTEGRITY_ERROR` |
| 8 | Quantity records of plate, beam, column (section 8) | not current: omitted and listed |

An element of a kind with no mapping keeps its place in the store untouched and appears in the file
as a proxy with the name, the placement, the mesh as published and the property set of section 9.
Its own attributes of unknown namespaces are not exported (L-01). It has no quantity set and its
mesh is exported as published, not re-verified against the kernel, because the kernel does not know
its kind; its face set has `Closed` unset.

### 4.3 Order

All elements are processed in ascending ElementId order (byte comparison of the lowercase text,
the order of F06 `ListElements`), levels in ascending LevelId order, materials in ascending
ContentDigest order (`IFCX-DET-001`).

## 5. Spatial structure and placement

### 5.1 Structure

The file holds one `IfcProject` (Name from the optional `project_name` argument, otherwise unset),
one synthetic `IfcSite` and one synthetic `IfcBuilding` (fixed names `Site` and `Building`, placements
at the world origin with identity axes) and one `IfcBuildingStorey` per stored Level
(`IFCX-LVL-001`), aggregated project to site, site to building, building to storeys. Levels belong to the model; the synthetic site and
building sit above them.

| Level attribute | IfcBuildingStorey attribute |
|-----------------|-----------------------------|
| name | `Name` |
| `elevation_mm` | `Elevation`, and `ObjectPlacement` at `(0, 0, elevation_mm)` relative to the building placement |
| LevelId | `GlobalId` by section 3.5 (role `storey`) |

`CompositionType` is `.ELEMENT.`. A storey and the project name follow the label rule of section 9.1: over 255
characters, `Name` is truncated to 255 and the full text is in `Description` (`IFCX-PSET-004`). Each element with a level is contained in that storey; an element
with no level is contained in the building (`IFCX-LVL-003`); a project with no level has no storey. A storey with no exported element is still written, without a containment relationship (`IFCX-EMIT-001`, 11.2).
An element assigned to a LevelId that the revision does not carry refuses the export
(`IFCX-LVL-002`).

### 5.2 Element placement

`IFCX-LVL-004`, `IFCX-PLC-001`. The F04 pose maps element-local coordinates to world by
`world = R(q) * local + translation_mm`. The element's `ObjectPlacement` is an `IfcLocalPlacement`
with `PlacementRelTo` the **building** placement (not the storey's, so a level adds no arithmetic to
any element number) and `RelativePlacement` an `IfcAxis2Placement3D`:

| Attribute | Value |
|-----------|-------|
| `Location` | `translation_mm` as an `IfcCartesianPoint` (millimetres) |
| `Axis` | `R(q)` applied to `(0, 0, 1)`, the third column of `R(q)` |
| `RefDirection` | `R(q)` applied to `(1, 0, 0)`, the first column of `R(q)` |

The second axis is implied (`Axis` cross `RefDirection`), so the element-local frame becomes the IFC
local frame exactly. `q` is normalized first (`IFCX-PLC-004`, F04 section 3); `q` and `-q` give
identical numbers because `R` is built from products of components, and negative zero is written
as zero. The direction values are written as computed in binary64; F04's dimensionless comparison
(absolute 1e-6 at zero, relative 1e-6 otherwise) applies to checks. A stored quaternion of a plate,
beam or column outside the F04 caps refuses the export (`IFCX-PLC-003`); a proxy whose placement is
not a rigid pose is exported without placement and representation (`IFCX-PLC-002`). Nothing is baked
into the geometry: the mesh is written in element-local coordinates and the placement moves it.

Note: the engine refuses a placement rotation other than the identity today
(`UNSUPPORTED_GEOMETRY`), so exported files have identity axes. The rule is specified now so that
it does not change when rotation is supported.

## 6. Geometry: the published mesh

### 6.1 Choice: the tessellated mesh only

The geometry is the tessellated mesh and nothing else; an extruded solid with a profile is not
written.

| Criterion | Tessellated face set (chosen) | Extruded solid (not used) |
|-----------|-------------------------------|----------------------------|
| Source | The mesh BeamCraft already publishes and verifies in the derived layer; the file shows what a USD reader shows | A second description derived from authored parameters |
| Holes and later operations | Part of the mesh with no extra entity | Profile voids now, booleans or openings later |
| Kinds | Every element with a mesh, including a proxy | Only kinds whose parameters the exporter understands |
| Parameters (width, height, thickness, axis, roll) | Lost (L-04) | Preserved |
| Size | 8 vertices and 12 triangles at least | A few entities |

The IFC4 form is `IfcTriangulatedFaceSet` (S28): a coordinate list (`IfcCartesianPointList3D`, S29)
and a list of vertex-index triples, with an optional `Closed` flag. It is chosen over
`IfcPolygonalFaceSet` because the published mesh consists of triangles only (F06 section 6: every
`faceVertexCounts` entry is 3), so no face needs a polygon form, and over a brep because the mesh
is not a boundary representation with topology. The shape representation has identifier `Body` and
type `Tessellation` (S25, S24).

**Losses stated (section 12):** the authored parameters of the shape (plate contour and thickness,
member axis, profile, roll) and the extrusion intent are not in the geometry; a reader gets
triangles. They survive only as the properties of section 9 (kind, hole count, profile digest and,
for a member, the declared values).

### 6.2 Mapping (`IFCX-GEO-001`, `-002`, `-003`)

| IFC attribute | Value |
|---------------|-------|
| `IfcShapeRepresentation` | `RepresentationIdentifier = 'Body'`, `RepresentationType = 'Tessellation'`, `Items = (IfcTriangulatedFaceSet)`, context the Body sub-context |
| `Coordinates` | `IfcCartesianPointList3D`: the mesh points in their stored order, element-local, millimetres, unscaled |
| `Normals` | unset (consumers compute them from the winding; the stored uniform normals are not exported) |
| `Closed` | `.T.` for plate, beam and column (the shell is certified closed, F06 section 6); unset for any other kind |
| `CoordIndex` | the stored triangles in their stored order, vertex indices **plus one** (IFC indices are 1-based, the derived layer's are 0-based) |
| `PnIndex` | unset |

The winding is as published: counter-clockwise seen from outside (`rightHanded`), so the right-hand
normal points out of the body, which is the orientation the IFC4 definition requires for a closed
face set. Vertex order and triangle order are not changed, merged, welded or re-sorted; the order of
the F06 canonical box (vertex `i = xb + 2*yb + 4*zb`, face quads in the F06 table order, quad
`(a,b,c,d)` as triangles `(a,b,c)` and `(a,c,d)`) is therefore the order of the file for plate, beam
and column without holes (case `E01-POS-16`).

**Number text.** The derived layer stores mesh points as binary32 (`point3f`). The exporter writes each coordinate as the **shortest decimal text that parses back
to the same binary32** (section 11.3), not as the longer text of the binary32 value widened to
binary64. A reader that parses the text as a double obtains a value within one binary32 unit of the
stored one. For integer millimetre coordinates the value is exact. Mesh coordinates stay binary32, the
precision of the derived layer.

A face with a vertex count other than 3, a mesh with no faces or an element with no derived mesh
(valid non-rectangular plates, members with another profile kind, kinds the kernel does not know)
gives an element **without a representation**, listed with the strict-mode code `UNSUPPORTED_GEOMETRY`
(`IFCX-GEO-004`); the exporter does not triangulate foreign polygons.

### 6.3 Mesh checks

`IFCX-GEO-005`. For a plate, beam or column the stored derived mesh is compared with the kernel's
re-evaluation of the authored inputs, as the publish-time verifier does (digest equality), so a
layer edited behind BeamCraft's back cannot reach the file as a certified body. `IFCX-GEO-006`: a
mesh with an index out of range, a non-finite coordinate or disagreeing counts refuses the export.
Counts are bounded by `IFCX-LIM-005`.

### 6.4 Independent check

`IFCX-GEO-008`. An independent check recomputes the volume of the written face set from the file
(the divergence sum `(1/6) * sum(v0 . (v1 x v2))` of F06 section 6) and the closed-shell property
(every edge in exactly two triangles, once in each direction) and compares the volume with the
published volume. The bound is the CR-TOL-001 quantity bound (relative 1e-6) when all coordinates
are exactly representable in binary32, as in the examples of the case file. For a general mesh the
bound is the one implied by half a binary32 unit per coordinate and is stated per case; it
is not claimed to be 1e-6 in general (binary32 coordinates). `IFCX-GEO-007`: no profile, extrusion, boolean or
opening entity exists in the file.

## 7. Material and profile

**Material** (`IFCX-MAT-001` to `IFCX-MAT-003`). A pinned material definition (`bim:material`) used
by at least one exported element becomes one `IfcMaterial` (S30). `Name` is the declared grade
or, when none is declared, the text `Unspecified`; `Description` is the declared standard
when present. A grade over 255 characters is truncated to 255 in `Name`, listed, and
the full grade is the property `FullName` (`IfcText`) of `BeamCraft_Material` (`IFCX-PSET-004`). Two different materials with no grade therefore have the same
name and are told apart by the property **`MaterialDigest`** (`IfcIdentifier`, the full 64 lowercase
hexadecimal digits) in an `IfcMaterialProperties` named `BeamCraft_Material` (S32), which every
material has. One `IfcRelAssociatesMaterial` (S31) links the material to all its exported elements
in ElementId order. A declared density becomes `IfcMaterialProperties` named `Pset_MaterialCommon`
with the property `MassDensity` (`IfcMassDensityMeasure`, kg/m3, S32 and S49). An absent density writes
no property: it is never zero and never a catalogue value. A grade never selects a density.
An element with no pinned material gets no association.

**Profile.** No profile entity is written: the geometry is the mesh. A member's profile ContentDigest
is the property `ProfileDigest` (section 9). The profile definition is nevertheless resolved
(`IFCX-MAT-004`): an unresolvable digest refuses the export, because the declared values below read
from it. Its **declared section values** (designation, standard, area, mass per metre,
surface per metre) are exported in the separate set `BeamCraft_Declared` (section 9.2), never in a quantity set.

## 8. Quantities

### 8.1 Rule

`IFCX-QTY-001`. Only **published current quantities** are exported. A quantity is written only when
its stored record exists, its signature equals the one the engine computes from the present authored
inputs, the stored value and status equal the engine's own computation and the status is `current`
(a forged value under a matching signature is stale). Nothing is
recomputed at export time. The exporter may call the kernel to verify a mesh (section 6.3) but never
to produce a quantity the store lacks.

| Freshness | Meaning (F04 section 5) | Written? |
|-----------|-----------------------------|----------|
| `current` | Record present, signature and value match | yes |
| `stale` | Record present, signature or value differ | no value; status in `BeamCraft_Element`; omission listed |
| `missing` | No record stored | no value; status `missing`; omission listed |
| `unknown` | Required input absent (density) | no value, never zero; status `unknown`; omission listed |
| `unsupported` | Geometry outside the scope | no value; status `unsupported`; omission listed |

A quantity omission is a **warning** (`IFCX-QTY-002`) and does not fail the export, including in
strict mode. A `current` record with a non-finite value, or a volume that is
not greater than zero, is `INTEGRITY_ERROR` and refuses the export (`IFCX-QTY-005`). A proxy has no
quantity set.

### 8.2 Mapping of the published quantities (IFC4)

Published today: `volume_mm3` and `mass_kg` with their statuses, shape and mass signatures
and an algorithm label. Units: volume is divided by `1e9` to cubic metres, as F06 section 2 does;
mass is already kilograms.

| BeamCraft quantity | Element kinds | IFC quantity (set) | Rule |
|--------------------|---------------|--------------------|------|
| Volume (body as authored, including holes) | beam, column, plate | `NetVolume` (`IfcQuantityVolume`) in `Qto_BeamBaseQuantities`, `Qto_ColumnBaseQuantities`, `Qto_PlateBaseQuantities` (S42 to S44) | always when current |
| Same volume | same | `GrossVolume` in the same sets | only for an element without operations: a plate without holes, and every member (`IFCX-QTY-004`) |
| Mass (volume times density) | beam, column | `NetWeight` (`IfcQuantityWeight`) in the beam and column sets | when current; kilograms |
| Same mass | beam, column | `GrossWeight` | same condition as `GrossVolume` |
| Mass | plate | property `NetWeightKg` (`IfcMassMeasure`) in `BeamCraft_Element` | the IFC4 plate set has no weight quantity (`IFCX-QTY-007`); when current |
| Same mass | plate | property `GrossWeightKg` | same condition as `GrossVolume` |

Each `IfcElementQuantity` is named exactly as the buildingSMART set (`Qto_...`). A plate with holes
has no `GrossVolume` and no `GrossWeight`, because the volume of the plate without its holes is not
a published record and the exporter does not recompute it.

Not published today and therefore not exported (L-06): beam `Length`, `CrossSectionArea`,
`OuterSurfaceArea`, `GrossSurfaceArea`, `NetSurfaceArea`, plate `Width`, `Perimeter`, `GrossArea`,
`NetArea`, and the fields `lengthMm`, `cutLengthMm`, `nominalMassKg`, `surfaceAreaM2`.
When the store publishes one of them as a current record, a contract version adds its row; the rule
"published current only" does not change.

### 8.3 Provenance

`IFCX-QTY-006`. `BeamCraft_Element` carries the volume and mass statuses, the shape and mass
signatures (64 lowercase hex digits) and the algorithm label of the records, so a reader can
compare the file with the derived layer. These signatures are the quantity records' own, not the
geometry keys.

## 9. Property sets

### 9.1 `BeamCraft_Element`

One `IfcPropertySet` per exported element, linked by one `IfcRelDefinesByProperties`
(`IFCX-PSET-001`). Every property is an `IfcPropertySingleValue`; names are fixed here.

| Property | Type | Content | Present |
|----------|------|---------|---------|
| `ElementId` | `IfcIdentifier` | Canonical 36-character ElementId | always |
| `ElementKind` | `IfcText` | `plate`, `beam`, `column`, or the kind token of a proxy | always |
| `SchemaVersion` | `IfcText` | `bim:schemaVersion` | always |
| `GeometryMode` | `IfcText` | `bim:geometryMode` | always |
| `ProfileDigest` | `IfcIdentifier` | Full ContentDigest of the profile definition | members |
| `MaterialDigest` | `IfcIdentifier` | Full ContentDigest of the material definition | with a material |
| `HoleCount` | `IfcInteger` | Number of through holes | plates |
| `VolumeStatus`, `MassStatus` | `IfcLabel` | `current`, `stale`, `missing`, `unknown`, `unsupported` | plate, beam, column |
| `ShapeSignature`, `MassSignature` | `IfcIdentifier` | Signatures of the stored records | when a record exists |
| `QuantityAlgorithm` | `IfcText` | Algorithm label of the stored record | when a record exists |
| `NetWeightKg`, `GrossWeightKg` | `IfcMassMeasure` | Plate mass, kilograms (section 8.2) | plates, when current |
| `Label` | `IfcText` | The full label | only when the name was truncated |

`IfcRoot.Name` is `bim:name` (S16 to S19). **Every BeamCraft string that lands in an `IfcLabel`
attribute is limited to 255 characters (S04), while the model allows labels of 256 bytes (names,
grades, designations, standards) and, in loaded content, kind tokens of up to 4096 bytes** (F04
section 3). The policy
(`IFCX-PSET-004`) is: the attributes that are `IfcLabel` are `IfcRoot.Name` (element, storey, project),
`IfcObject.ObjectType` and `IfcMaterial.Name`; a longer text is truncated to its first 255 code
points, the truncation is listed (`IFC_NAME_TRUNCATED`) and the full text is kept in an unbounded
`IfcText`: the property `Label` for an element, `FullName` in `BeamCraft_Material` for a material,
`Description` for a storey and the project. For a proxy `IfcObject.ObjectType` is the kind token, truncated
the same way, and `ElementKind` holds it in full. Every other BeamCraft string is written as `IfcText`
(no limit); identifiers and digests are at most 64 characters. Nothing is refused for length.
No custom set name starts with `Pset_` or `Qto_` (`IFCX-PSET-002`), because those prefixes belong
to the specification's own sets (`Pset_MaterialCommon` and the `Qto_` base quantity sets are the
only ones used). The actor id (`bim:createdBy`), the revision number of the element, file paths, host
names and any time are not written (`IFCX-PSET-003`; L-08). The project-level data (ProjectId,
RevisionId, contract and exporter versions) go in the report, not in a property set.

### 9.2 `BeamCraft_Declared`

For a member whose profile definition carries declared section values, one more
`IfcPropertySet` with its `IfcRelDefinesByProperties` (`IFCX-DECL-001`, `-002`). It is absent when
the definition declares nothing. Values are written **as authored**, without conversion or check
against any formula; the unit is in the property name, and every property has a `Description`
saying that the value is declared in the profile definition and not computed by BeamCraft.

| Property | Type | Content |
|----------|------|---------|
| `DeclaredDesignation` | `IfcText` | declared designation |
| `DeclaredStandard` | `IfcText` | declared standard |
| `DeclaredSectionAreaMm2` | `IfcReal` | declared area in mm2 |
| `DeclaredMassPerMeterKg` | `IfcReal` | declared mass per metre in kg |
| `DeclaredSurfacePerMeterM2` | `IfcReal` | declared surface per metre in m2 |

These are catalogue-style values the author typed in, so they can disagree with the stored
quantities; they never replace them and are never put in a `Qto_` set.

## 10. Units

`IFCX-UNIT-001`. One `IfcUnitAssignment` on the project (S39 to S41):

| Unit type | Definition |
|-----------|-----------|
| `LENGTHUNIT` | `IfcSIUnit(.LENGTHUNIT., .MILLI., .METRE.)` |
| `PLANEANGLEUNIT` | `IfcSIUnit(.PLANEANGLEUNIT., $, .RADIAN.)` |
| `AREAUNIT` | `IfcSIUnit(.AREAUNIT., $, .SQUARE_METRE.)` |
| `VOLUMEUNIT` | `IfcSIUnit(.VOLUMEUNIT., $, .CUBIC_METRE.)` |
| `MASSUNIT` | `IfcSIUnit(.MASSUNIT., .KILO., .GRAM.)` |
| `MASSDENSITYUNIT` | `IfcDerivedUnit` of kilogram to the power 1 and metre to the power -3 |

`IFCX-UNIT-002`. The internal unit is the millimetre (F04 section 3), so **no length is scaled**:
the number in the file is the internal number, and the storey elevation is written as stored. Only
volume is converted, from mm3 to m3, because the quantity sets are in the project's volume unit.
The area and angle units are declared for completeness; no area or angle value is written in 0.1.0
(the declared areas of section 9.2 carry their unit in the name). The
`IfcGeometricRepresentationContext.Precision` is `0.001` (millimetres), the absolute geometric floor
of F04 section 3.

## 11. Determinism

`IFCX-DET-006`. The same revision, the same options and the same exporter version give a
byte-identical file and a byte-identical report. Nothing else influences them: no clock, random
source, locale, thread count, hash-map iteration order, environment variable, working directory or
output path (F06 section 2 applies the same rule to the kernel).

### 11.1 Ordering

Section 4.3. Within an element the entities are emitted in the fixed order of 11.2. Sets with no
natural order (the elements of a relationship) are sorted by ElementId.

### 11.2 Instance numbers

STEP instance names `#1`, `#2`, ... are assigned by one counter in this emission order
(`IFCX-DET-002`):

1. units, then the representation context and the Body sub-context;
2. the project, the site, the building, then the storeys in LevelId order, with their placements,
   and the aggregation relationships;
3. the materials in ascending digest order, each followed by its properties;
4. for each element in ElementId order: placement entities (when the element has a placement),
   point list, face set, shape representation and product definition shape (when it has a mesh), the
   product, `BeamCraft_Element` with its `IfcRelDefinesByProperties`, `BeamCraft_Declared` with its
   relationship (only with a declared value), and the quantity set with its relationship (**only
   with at least one quantity to write**);
5. the containment relationships (one per storey in LevelId order that has at least one exported
   element, then the building's if at least one exported element has no level), then one material
   association per material in digest order.

Forward references are allowed in a STEP file; the counter order, not the reference order, is fixed.
An optional entity is written only when it has content (`IFCX-EMIT-001`): a relationship never has
an empty related set or a missing relating definition, and no list attribute is empty. In step 3 a
material is written only if an exported element uses it. In step 2 the building-to-storeys
aggregation is written only when there is at least one storey.

### 11.3 Real numbers

`IFCX-DET-003`, `IFCX-GEO-003`. A REAL is the shortest decimal digit string `d1 d2 ... dn` that
parses back to the same value **in its own type** (ties to the value closest to the exact one), with
scientific exponent `x` (value = `d1.d2...dn` times 10 to the `x`). The type is binary64 for every
number the exporter computes (placement vectors, quantities, elevation, declared values) and
binary32 for mesh coordinates, which are stored as binary32. Notation:

- zero, including negative zero, is `0.`; a negative value has a leading `-`;
- if `-6 <= x <= 14` the notation is positional: digits, zero fill if needed, and a decimal point,
  with at least the point after an integral value (`1000.`, `0.5`, `0.000001`);
- otherwise the notation is `d1.d2...dn` followed by `E` and the exponent without `+` and without
  leading zeros (`1.E15`, `1.25E-9`).

There is never a locale separator, a leading `+`, a bare exponent without a point, or a NaN or
infinity (those are rejected before writing). Vectors are in the case file (`E01-POS-26` for
binary64, `E01-POS-44` for binary32).

### 11.4 Strings and encoding

`IFCX-DET-004`. The file is 7-bit ASCII with LF line ends. In a string, an apostrophe is doubled and
a backslash is doubled. Each maximal run of non-ASCII characters is one segment: `\X2\` followed by
four upper-case hex digits per UTF-16 unit and `\X0\` when the run is all in the basic multilingual
plane, otherwise `\X4\` followed by eight upper-case hex digits per code point and `\X0\`. Control
characters cannot occur (F04 label rule). These are the escapes of the STEP physical file as
described for IFC (S02); acceptance confirms them against an independent reader (L-14).

### 11.5 Header

`IFCX-DET-005`. The fixed header, with the exporter version `V` and the time stamp `T`:

```text
ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('BeamCraft IFC export profile 0.1.0'),'2;1');
FILE_NAME('beamcraft-export.ifc','T',(''),(''),'BeamCraft IFC writer V','BeamCraft V','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
```

`T` is the UTC calendar time of the 48-bit millisecond timestamp inside the RevisionId, truncated to
whole seconds, in the form `YYYY-MM-DDTHH:MM:SS` (a pure function of the revision). `FILE_NAME`'s file name is the fixed text shown, not the real path. `V` is the exporter's crate
version, so byte identity is claimed per exporter version. `FILE_DESCRIPTION` names no model view
(section 2.1). The project-level provenance (ProjectId, RevisionId, contract version) is the
report's, not the file's.

### 11.6 Platform

`IFCX-DET-007`. The exporter uses only `+ - * /` and `sqrt` except where the kernel derived a mesh
with a nonzero member roll, which uses `sin` and `cos`. Cross-target byte identity for rolled members
is therefore not claimed until the sin/cos reproducibility question of the member frame is
resolved. For all other elements it is claimed, as in F06 for the meshes.

## 12. Losses

The export is lossy by design. The file never claims to be a complete copy of the project. Each loss
is listed in the report when it applies to an element and is stated here once.

| ID | Loss |
|----|------|
| L-01 | Deactivated elements are not exported (`IFCX-SCOPE-005`). Elements of kinds with no mapping, including reserved and `reference` kinds, are exported as proxies with name, placement, mesh and `BeamCraft_Element` only; their own attributes, namespaces and relationships stay in the store (`IFCX-SCOPE-001`). |
| L-02 | Elements with no usable derived mesh (valid non-rectangular plates, members with another profile kind, meshes with non-triangle faces) are exported without a representation (`IFCX-GEO-004`). |
| L-03 | A hole is part of the mesh. Its polygon, its intent (a round hole made by a generator is a polygon of N sides) and the hole count other than as `HoleCount` are not exported. |
| L-04 | The authored shape parameters (plate contour and thickness, member axis, profile width and height, roll) and the extrusion intent are not exported; only the mesh is. FeatureIds (`fid1`), the face table, the mesh digest, the shape key and the dependency keys are not exported. No `IfcShapeAspect` is written. |
| L-05 | The stored face normals and the mesh extent are not exported. Mesh coordinates carry the binary32 precision of the derived layer. |
| L-06 | Quantities that are not current are absent (stale, missing, unknown). Quantities that no record publishes are absent (section 8.2). Declared section values are exported as declared properties and are not quantities. |
| L-07 | A profile is reduced to its digest and its declared values. There is no `IfcBeamType`, no material profile set, no catalogue reference. |
| L-08 | Authorship (`bim:createdBy`), per-element revision, workspace and draft state, the journal, undo and redo history and every time are not exported. |
| L-09 | Relationships between elements (F10 section 5) are not exported; none exist in the first slice. |
| L-10 | Display appearance and colours are not exported. |
| L-11 | A text longer than 255 characters in an `IfcLabel` attribute (element, storey, project or material name, object type) is truncated to 255 code points; the full text is in `Label`, `FullName` or `Description` (`IFCX-PSET-004`). |
| L-12 | The site and the building are synthetic. The export records no grids (the [grid contract draft](t01-project-grids-draft-v0.1.md) keeps them in the model, skipped by the exporter), no site data and no building data. |
| L-13 | The quaternion is not recoverable: only the rotation is, and only up to the rounding of the direction vectors (`q` and `-q` are indistinguishable). |
| L-14 | The escapes of 11.4 and the header form are taken from the public description of the STEP physical file and are not yet checked against an independent reader. |
| L-15 | No model view definition is claimed (section 2.1); a consumer that requires one may reject the file. |
| L-16 | The storey placement is the point `(0, 0, elevation)`; element placements are relative to the building, not to the storey, so a reader that expects element placement relative to its storey sees world-aligned values. |

## 13. Diagnostics and codes

The export returns one result with the envelope of F10 section 6.3 (status named `ok`,
`ok_with_losses`, `failed` or `incomplete`; `primary_code`; `total_findings`; `truncated`; at most
256 findings) and the ordering of F10 section 6.4 (severity, then code, then ElementId, then field,
bytewise). `IFCX-ERR-001`: unavailable input makes the status `incomplete` and the primary code
`DEPENDENCY_UNAVAILABLE`, ahead of any other error. A failed or incomplete export writes no file. A
finding has `code` (the `IFC_` token), `f04_code`, `severity`, an optional `element_id` and a fixed
schema `field` token; messages are fixed English text that never echo input values, private paths or
exceptions (the F06 section 3 rule). The `f04_code` of a finding is `null` unless the finding is
fatal in strict mode (see below).

| `IFC_` code | F04 code | Severity / level | Meaning | Rules |
|-------------|----------|------------------|---------|-------|
| `IFC_VERSION_UNSUPPORTED` | `UNSUPPORTED_VERSION` | error, request | Contract version or schema identifier not supported | VER-001 |
| `IFC_FORMAT_UNSUPPORTED` | `UNSUPPORTED_PROPERTY` | error, request | Output format other than STEP | VER-002 |
| `IFC_IMPORT_UNSUPPORTED` | `UNSUPPORTED_COMMAND` | error, request | Import or any IFC reading requested | SCOPE-003 |
| `IFC_ARGUMENT_INVALID` | `INVALID_ARGUMENT` | error, request | Missing, unknown, repeated or out-of-range option; existing or link output name; quaternion outside the caps | REQ-002, ERR-003, PLC-003 |
| `IFC_ID_INVALID` | `INVALID_ID` | error, request | Malformed RevisionId or ElementId | REQ-001, GUID-002 |
| `IFC_NOT_FOUND` | `NOT_FOUND` | error, request | Explicit revision does not exist; pinned definition does not resolve | REQ-003, MAT-004 |
| `IFC_INPUT_UNAVAILABLE` | `DEPENDENCY_UNAVAILABLE` | error, request | Revision, definition or index cannot be read completely; no stored ProjectId | SCOPE-002, PRJ-001, ERR-001 |
| `IFC_LEVEL_DANGLING` | `DANGLING_RELATIONSHIP` | error, request | Element assigned to a level the revision does not carry | LVL-002 |
| `IFC_ID_DUPLICATE` | `DUPLICATE_ID` | error, request | Two elements with one ElementId | GUID-005 |
| `IFC_INTEGRITY` | `INTEGRITY_ERROR` | error, request | Repeated GlobalId; stored mesh differs from re-evaluation; invalid mesh; current record with a bad value | GUID-006, GEO-005, GEO-006, QTY-005 |
| `IFC_LIMIT` | `LIMIT_EXCEEDED` | error, request | Too many elements, levels, mesh vertices or triangles, or output bytes | LIM-001, LIM-002, LIM-005 |
| `IFC_OUTPUT_FAILED` | `BACKEND_UNAVAILABLE` | error, request | The output sink or file system failed | ERR-002 |
| `IFC_CANCELLED` | `CANCELLED` | error, request | Cancelled before the install point | ERR-004 |
| `IFC_KIND_PROXY` | none | warning, element | Kind without mapping, exported as proxy | SCOPE-001 |
| `IFC_GEOMETRY_ABSENT` | `UNSUPPORTED_GEOMETRY` (strict) | warning, element | No usable mesh; exported without representation | GEO-004 |
| `IFC_PLACEMENT_ABSENT` | `UNSUPPORTED_GEOMETRY` (strict) | warning, element | Proxy placement is not a rigid pose | PLC-002 |
| `IFC_QUANTITY_OMITTED` | none | warning, element | Quantity stale, missing, unknown or unsupported | QTY-002 |
| `IFC_NAME_TRUNCATED` | none | warning, element, level or project | Text longer than 255 characters written into an `IfcLabel` attribute | PSET-004 |
| `IFC_ELEMENT_DEACTIVATED` | none | info, element | Deactivated element not exported | SCOPE-005 |

Every F04 code used above exists in F04 section 8; no code is added. **Strict mode** (`--strict`,
`IFCX-ERR-005`) is kept as an optional flag because the default is to export everything and report.
In strict mode **only an element-level finding that carries an F04 code is fatal**: `IFC_GEOMETRY_ABSENT`
and `IFC_PLACEMENT_ABSENT` (both `UNSUPPORTED_GEOMETRY`), so a caller can demand that every plate,
beam and column leaves with its mesh. The export is refused with the code of the first such finding
in finding order (F10 section 6.4). Findings without a code (`IFC_KIND_PROXY`, `IFC_QUANTITY_OMITTED`,
`IFC_NAME_TRUNCATED`, `IFC_ELEMENT_DEACTIVATED`) are never fatal, strict or not.

**Status.** `failed` or `incomplete` as above; otherwise `ok_with_losses` when any warning was raised
**or any element was not exported** (a deactivated element, an info finding), and `ok` only when
nothing was raised and every element was exported. A missing mass because no material is pinned is
a quantity omission and therefore a warning.

## 14. Bounds

| Item | Bound | Origin |
|------|-------|--------|
| Elements per file | 65,536 (`IFCX-LIM-001`), checked before any output | the Pick scan bound of F06 section 9 |
| Levels per project | 1,024 (`IFCX-LIM-005`) | own limit |
| One mesh | 262,144 vertices and 524,288 triangles (`IFCX-LIM-005`) | own limit; covers a plate of 64 holes of 1024 points |
| Output size | 268,435,456 bytes (256 MiB), checked before each write with checked arithmetic (`IFCX-LIM-002`) | own limit |
| Findings in the report | 256 listed, total counted up to 1,000,000 | F10 section 6.3 |
| Label | 1 to 256 bytes at input (F04); 255 characters in an `IfcLabel` attribute, the rest preserved in an `IfcText` | F04 section 3, S04 |
| Strings | at most 4096 bytes before escaping (F04); the escaped text counts toward the output cap | F04 section 3 |
| Instance numbers | a checked counter; overflow is `LIMIT_EXCEEDED` | the output cap makes it unreachable |

The writer allocates only for validated, capped input, uses no recursion, indexes nothing by an
input-derived position and writes through a bounded sink (`IFCX-LIM-003`, `AGENTS.md` section 4). A
degraded element never panics; a request failure leaves no file.

**Output publication.** The writer streams to a temporary file in the destination directory,
checks the running size against the cap, flushes and syncs it, and installs it under the final name
only after every check passed. An existing file is not replaced unless the caller asks for it, and
a symbolic link or Windows reparse point at the output name or its parent is refused, as the byte
storage refuses them (`IFCX-ERR-002`, `IFCX-ERR-003`). A failure removes the temporary file.
No network access is involved; the output path is a local I/O-edge argument and authorises nothing.

## 15. Headless surface and writer design

### 15.1 Where it lives

| Piece | Path |
|-------|------|
| Pure IFC writer: GlobalId, STEP lexical rules, entity templates, bounded sink | `crates/ifc` (`beamcraft-ifc`, layer 2, no file system, no third-party dependency) |
| Atomic output (temporary file, sync, non-replacing install) | `beamcraft_host::output` |
| Export adapter and the commands `export-ifc` and `ifc-guid` | `beamcraft_commands::export` |

### 15.2 Writer design

1. **`guid`**: `encode(&Uuid7) -> [u8; 22]`, `decode(&str) -> Result<[u8; 16], _>`, `synthetic(role,
   keys) -> [u8; 22]`. Pure functions with the vectors of section 3.2 as tests.
2. **`step`**: the lexical layer: `real64(f64)`, `real32(f32)` per 11.3, `string(&str)` per 11.4, an
   instance counter, and a `BoundedSink` that fails when the byte cap would be exceeded.
3. **`model`**: the plain input structure the engine builds: project data (the stored ProjectId,
   optional name), unit constants, levels, materials, and a sorted list of element records that
   already contain **resolved data** (placement vectors, the mesh points and 1-based triangles,
   quantities with their freshness, names, digests, declared values, level). The writer derives no
   geometry and calls no kernel.
4. **`emit`**: a fixed sequence of entity templates in the order of 11.2. There is no generic IFC
   object graph, no schema table and no reflection; adding an entity is adding a template and a case.
5. **`report`**: the findings, the per-element table and counts, assembled by the engine from the
   writer's element results.

Tests are the case file plus unit tests of `guid` and `step`. The independent check of a produced
file is an acceptance check (section 19).

### 15.3 Engine API (logical)

`export_ifc(snapshot, request, sink) -> ExportReport` in `beamcraft_engine::export`, with `request`
holding: `contract_version`, `schema`, `format`, optional `revision_id`, optional `project_name`,
`strict`, `overwrite`. The ProjectId and the levels come from the pinned snapshot. It reads only
that snapshot, never writes to the store, returns the report of 15.4 and writes the file only
through the sink. Two pure helpers are exposed: `ifc_global_id(&ElementId) -> GlobalId` and
`decode_global_id(&str) -> Result<[u8; 16], IdError>` (sections 3.1 and 3.4). The Rust names are
not fixed by this contract.

### 15.4 CLI and report

```text
beamcraft export-ifc --store ROOT --out FILE
  [--revision UUID] [--project-name TEXT] [--schema IFC4] [--format step]
  [--strict] [--overwrite] [--report FILE]
beamcraft ifc-guid --id UUID          prints the GlobalId (INVALID_ID for a malformed id)
beamcraft ifc-guid --decode GLOBALID  prints the UUID text, or INVALID_ARGUMENT
```

There is no `--project-id`: it is read from the project entry (`IFCX-REQ-002`, case `E01-NEG-34`).
The CLI parses, calls the engine API and prints the report as JSON with sorted keys; it computes
nothing else (`IFCX-SURF-001`). The report contains:

| Field | Content |
|-------|---------|
| `contract_version`, `schema`, `exporter_version` | strings |
| `project_id`, `revision_id` | UUID text |
| `status`, `primary_code`, `incomplete`, `total_findings`, `truncated`, `findings` | F10 envelope, section 13 |
| `counts` | `elements_total`, `exported`, `not_exported`, `proxies`, `without_representation` |
| `levels` | `level_id`, `ifc_global_id` per level |
| `elements` | one entry per element of the revision in ElementId order: `element_id`, `ifc_global_id` (null when none), `kind`, `exported`, `entity` (`IfcBeam`, ... or `IfcBuildingElementProxy`), `representation` (`Tessellation` or null), `volume`, `mass` (freshness tokens), `omissions` |
| `output` | `bytes` and `sha256` of the file (null when no file) |

It holds no path, no host name and no time (`IFCX-SURF-002`). The file and the report are the only
outputs; the exit code follows the existing CLI convention.

## 16. Acceptance cases

[The case file](../../tests/fixtures/contracts/e01-ifc-export-cases-v0.1.json) holds 105 original logical acceptance
descriptions: 52 positive and 53 negative, with stable IDs `E01-POS-nn` and `E01-NEG-nn`, and
78 rules `IFCX-<FAMILY>-<NNN>` of which every one is exercised by at least one case. A
negative case is one where the rule fires: an element is exported with a listed loss, a warning is
raised, or the request is refused with the stated F04 code. A case with a `needs` field depends on
a limit that is not yet lifted or a harness hook. Acceptance runs them against the real exporter
and a real store, with an independent check of the written file.

| Topic | Cases |
|-------|-------|
| GlobalId encoding, decoding, ordering, synthetic ids, stability, no persistence | `E01-POS-01` to `E01-POS-15`, `E01-POS-47`, `E01-NEG-01` to `E01-NEG-14` |
| Plate, beam, column, hole, rotation, material, quantities, declared values | `E01-POS-16` to `E01-POS-22`, `E01-POS-28`, `E01-POS-29`, `E01-POS-40` to `E01-POS-43`, `E01-POS-48` |
| Determinism, REAL and string text, units, header | `E01-POS-23` to `E01-POS-27`, `E01-POS-39`, `E01-POS-44` |
| Levels and project | `E01-POS-45`, `E01-POS-46`, `E01-NEG-48`, `E01-NEG-49` |
| Proxies, elements without mesh, scope, headless, report, validation gate | `E01-POS-30` to `E01-POS-38`, `E01-POS-49`, `E01-POS-50`, `E01-NEG-15` to `E01-NEG-17`, `E01-NEG-36`, `E01-NEG-43`, `E01-NEG-50` |
| Integrity, quantities freshness | `E01-NEG-18` to `E01-NEG-29` |
| Label limits, optional entities | `E01-POS-51`, `E01-POS-52`, `E01-NEG-51` to `E01-NEG-53` |
| Request errors, limits, output failures | `E01-NEG-30` to `E01-NEG-35`, `E01-NEG-37` to `E01-NEG-42`, `E01-NEG-44` to `E01-NEG-47` |

### 16.1 Rule catalogue

Severity `error` rules name the F04 code of the failing condition; `mapping` rules have no runtime
diagnostic and are checked by the acceptance cases. Counts: 78 rules, 26 error, 5 warning, 3 info,
44 mapping.

| Rule | Severity | F04 code | Statement |
|------|----------|----------|-----------|
| IFCX-SCOPE-001 | warning | - | An element whose kind has no IFC mapping (every kind except plate, beam and column, reserved or unknown) is exported as IfcBuildingElementProxy and listed. It is never skipped, and it is not fatal in strict mode. |
| IFCX-SCOPE-002 | error | DEPENDENCY_UNAVAILABLE | The export reads exactly one verified committed revision, the published head by default or an explicit RevisionId. A revision that cannot be read and verified completely is refused; there is no partial export. |
| IFCX-SCOPE-003 | error | UNSUPPORTED_COMMAND | IFC import, and any reading of IFC by BeamCraft, is outside this contract. A request to import is refused. |
| IFCX-SCOPE-004 | error | INTEGRITY_ERROR | Export is read-only: it changes no authored state, derived state, revision or generation, and writes no attribute into the store. |
| IFCX-SCOPE-005 | info | - | An element with `bim:isDeactivated = true` is not exported and is listed (F10 section 3.2). It is the only element that is not exported. |
| IFCX-SCOPE-006 | mapping | - | Every other element of the revision is exported. An element without usable geometry is exported without a representation and listed; nothing else is skipped. |
| IFCX-VER-001 | error | UNSUPPORTED_VERSION | contract_version is exactly 0.1.0 and the requested schema identifier is IFC4 (IFC4 ADD2 TC1), the only supported one. |
| IFCX-VER-002 | error | UNSUPPORTED_PROPERTY | The only output format is the clear-text STEP physical file. Any other requested format is refused. |
| IFCX-VER-003 | info | - | No model view definition is claimed. The header names none and the file does not assert conformance to one. |
| IFCX-GUID-001 | mapping | - | The GlobalId of an element is E(ElementId) of section 3.1: alphabet, first character 0 to 3, big-endian bit order, 22 characters. |
| IFCX-GUID-002 | error | INVALID_ID | An ElementId that is not a canonical lowercase UUIDv7 with the RFC variant gets no GlobalId, with no repair, lowercasing, truncation, padding or hash fallback. In an export the element makes the request fail: a stored revision with such an id is not a valid revision. |
| IFCX-GUID-003 | error | INVALID_ARGUMENT | The decoder accepts exactly 22 characters of the alphabet whose first character is 0, 1, 2 or 3. Everything else is rejected. |
| IFCX-GUID-004 | mapping | - | Entities that are not elements get a synthetic GlobalId from the SHA-256 construction of section 3.5, a function of the stored ProjectId, the LevelId or ElementId and the fixed role only (owner decision Q-11). |
| IFCX-GUID-005 | error | DUPLICATE_ID | Two elements of the revision with the same ElementId refuse the export (F10 DV-BASE-002). |
| IFCX-GUID-006 | error | INTEGRITY_ERROR | Two entities of one file with the same GlobalId, including a synthetic one equal to an element one, refuse the export. |
| IFCX-GUID-007 | mapping | - | The GlobalId is a pure function of the ElementId. It does not change with rename, edit, placement, revision, export run, platform or time; a copy has a new ElementId and so a new GlobalId. |
| IFCX-GUID-008 | mapping | - | The GlobalId is computed at export (and by the CLI helper) and is not persisted: no attribute is written to the authored or derived layer. A stored attribute of that name found in a layer is unknown content, preserved, ignored and never trusted (owner decision Q-6). |
| IFCX-REQ-001 | error | INVALID_ID | An explicit revision_id is a full canonical UUIDv7 text. |
| IFCX-REQ-002 | error | INVALID_ARGUMENT | A required option is missing, an option is unknown or repeated, or a value is out of range. There is no project-id option: the ProjectId is read from the project entry. |
| IFCX-REQ-003 | error | NOT_FOUND | An explicit revision_id names no committed revision of the store. |
| IFCX-PRJ-001 | error | DEPENDENCY_UNAVAILABLE | The project entry of the revision must record a ProjectId. A revision without one is refused; no ProjectId is invented or taken from an option. |
| IFCX-PRJ-002 | mapping | - | The GlobalIds of the project, site, building, storeys and relationships derive from the stored ProjectId, so two exports of the same project agree and two projects never share them. |
| IFCX-LVL-001 | mapping | - | Each stored Level becomes one IfcBuildingStorey with its name and elevation; the elements assigned to it are contained in it by one IfcRelContainedInSpatialStructure. |
| IFCX-LVL-002 | error | DANGLING_RELATIONSHIP | An element assigned to a LevelId that no Level of the revision carries refuses the export. |
| IFCX-LVL-003 | mapping | - | An element with no level is contained in the IfcBuilding. A project with no Level has no storey. |
| IFCX-LVL-004 | mapping | - | A storey placement is the point (0, 0, elevation) relative to the building placement. Element placements stay relative to the building placement, so a level adds no arithmetic to any element number. |
| IFCX-PLC-001 | mapping | - | The element placement becomes one IfcLocalPlacement relative to the building placement: Location is translation_mm, Axis is R(q) applied to (0,0,1), RefDirection is R(q) applied to (1,0,0). |
| IFCX-PLC-002 | warning | UNSUPPORTED_GEOMETRY | A placement of a proxy element that cannot be read as a rigid pose (scale, shear, reflection, matrix form) is not exported: the element has no ObjectPlacement and no representation, and is listed. |
| IFCX-PLC-003 | error | INVALID_ARGUMENT | A stored quaternion of a plate, beam or column outside the F04 section 3 caps refuses the export: the revision violates F04. |
| IFCX-PLC-004 | mapping | - | The quaternion is normalized before use; q and -q give identical output; negative zero is written as zero. |
| IFCX-GEO-001 | mapping | - | Every element with usable geometry has exactly one shape representation, identifier Body, type Tessellation, with exactly one IfcTriangulatedFaceSet item and nothing else (owner decision Q-2). |
| IFCX-GEO-002 | mapping | - | The face set is the published derived mesh of the pinned revision in its stored vertex and triangle order, in element-local coordinates, with 1-based indices; Closed is true for plate, beam and column and unset for any other kind. Normals are not written. |
| IFCX-GEO-003 | mapping | - | Mesh coordinates are the stored binary32 values written as the shortest decimal that parses back to the same binary32, unscaled millimetres (section 11.3). |
| IFCX-GEO-004 | warning | UNSUPPORTED_GEOMETRY | An element that has no derived mesh (geometry status unsupported, for example a valid non-rectangular plate or a member with another profile kind), or whose mesh has a face that is not a triangle, is exported without a representation and listed. The code is the one strict mode refuses with. |
| IFCX-GEO-005 | error | INTEGRITY_ERROR | For a plate, beam or column the stored derived mesh must equal the kernel's re-evaluation of the authored inputs (digest equality). A mismatch refuses the whole export. |
| IFCX-GEO-006 | error | INTEGRITY_ERROR | A derived mesh that is structurally invalid (an index out of range, a non-finite coordinate, counts that disagree) refuses the export. |
| IFCX-GEO-007 | mapping | - | Holes are part of the mesh. No profile, extrusion, boolean, opening or void entity is written, and no authored parameter other than those in BeamCraft_Element and BeamCraft_Declared (section 9). |
| IFCX-GEO-008 | mapping | - | The volume of the exported face set, recomputed from the file by the divergence sum, equals the published volume within the bound of section 6.4. |
| IFCX-MAT-001 | mapping | - | Each distinct pinned material digest used by an exported element becomes one IfcMaterial, associated with all its elements by one IfcRelAssociatesMaterial. An element without a material gets no association and no invented default. |
| IFCX-MAT-002 | mapping | - | IfcMaterial.Name is the declared grade, otherwise the text Unspecified. Description is the declared standard when present. The full digest is always written as the property MaterialDigest of the property group BeamCraft_Material (owner decision Q-9). |
| IFCX-MAT-003 | mapping | - | MassDensity is written only when the definition authors a density, as received. An unknown density is never written as zero or any default. |
| IFCX-MAT-004 | error | NOT_FOUND | A pinned profile or material digest of an exported element that does not resolve to a definition refuses the export (F10 DV-BASE-006). |
| IFCX-QTY-001 | mapping | - | A quantity is written only from a stored record whose signature equals the freshly computed one and whose status is current. Nothing is recomputed at export time. |
| IFCX-QTY-002 | warning | - | A quantity whose freshness is stale, missing, unknown or unsupported is not written (no zero, no previous value); its status is written in BeamCraft_Element and the omission is listed (owner decision Q-5). |
| IFCX-QTY-003 | mapping | - | Mapping of the published quantities: volume to NetVolume, mass to NetWeight, in the IFC4 base quantity set of the product type and in SI units (section 8.2). |
| IFCX-QTY-004 | mapping | - | GrossVolume and GrossWeight are written, with the same values, only for an element without operations (a plate without holes, every member). Otherwise they are omitted. |
| IFCX-QTY-005 | error | INTEGRITY_ERROR | A stored record with status current whose value is not finite, or whose volume is not greater than zero, refuses the export. |
| IFCX-QTY-006 | mapping | - | BeamCraft_Element names the statuses, signatures and algorithm label of the records, so a reader can compare them with the derived layer. |
| IFCX-QTY-007 | mapping | - | The IFC4 plate base quantity set has no weight quantities. A plate's current mass is written as the properties NetWeightKg and GrossWeightKg of BeamCraft_Element (IfcMassMeasure, kilograms), under the same freshness and Gross rules. |
| IFCX-DECL-001 | mapping | - | Declared section values of a member's profile definition (designation, standard, area, mass per metre, surface per metre) are written, when present, in the separate property set BeamCraft_Declared, never in a quantity set (owner decision Q-10). |
| IFCX-DECL-002 | mapping | - | Declared values are written as authored, without conversion or check, in properties whose names carry the unit (DeclaredSectionAreaMm2, DeclaredMassPerMeterKg, DeclaredSurfacePerMeterM2), each with a description that says it is declared and not computed. |
| IFCX-PSET-001 | mapping | - | Each exported element has one property set BeamCraft_Element with the properties of section 9.1. |
| IFCX-PSET-002 | mapping | - | A BeamCraft property set or group name never starts with Pset_ or Qto_, except Pset_MaterialCommon and the Qto_ sets of the specification used as section 8 and 7 state. |
| IFCX-PSET-003 | mapping | - | No property, string or header field contains an actor id, credential, private path, host name, user name or wall-clock time. |
| IFCX-PSET-004 | warning | - | A BeamCraft string written into an IfcLabel attribute (IfcRoot.Name of an element, storey or project, IfcObject.ObjectType, IfcMaterial.Name) is limited to 255 characters. A longer text is truncated to 255 code points and the truncation is listed; the full text is kept in an unbounded IfcText: the property Label (element), FullName in BeamCraft_Material (material), IfcRoot.Description (storey, project) or ElementKind in BeamCraft_Element (proxy ObjectType). Other BeamCraft strings are written as IfcText, so no length limit applies to them. |
| IFCX-UNIT-001 | mapping | - | The unit assignment is: length milli metre, plane angle radian, area square metre, volume cubic metre, mass kilogram, mass density kilogram per cubic metre (derived unit). |
| IFCX-UNIT-002 | mapping | - | Lengths and coordinates are written exactly as the internal millimetre values (mesh values as their binary32 text). No scaling is applied to any length; the storey elevation is written as stored. |
| IFCX-DET-001 | mapping | - | Elements, levels and relationship members are processed and listed in ascending canonical id order (byte comparison of the 36-character lowercase text); materials in ascending digest order. |
| IFCX-DET-002 | mapping | - | STEP instance numbers are assigned by a counter in the fixed emission order of section 11.2. |
| IFCX-DET-003 | mapping | - | A binary64 REAL and a binary32 REAL are each written as the shortest decimal text that parses back to the same value of its own type, in the notation of section 11.3. |
| IFCX-DET-004 | mapping | - | Strings are escaped as in section 11.4. The whole file is 7-bit ASCII with LF line ends. |
| IFCX-DET-005 | mapping | - | Header fields are the fixed values of section 11.5; the only variable one is the time stamp derived from the RevisionId. |
| IFCX-DET-006 | mapping | - | The same revision, options and exporter version give a byte-identical file and report. No clock, random source, locale, thread count, hash-map order, environment variable or path influences them. |
| IFCX-DET-007 | info | - | Cross-platform byte identity is claimed only when no member has a nonzero roll; with a nonzero roll it depends on the unresolved sin/cos reproducibility question (F06 section 2). |
| IFCX-ERR-001 | error | DEPENDENCY_UNAVAILABLE | The export result uses the F10 section 6.3 envelope precedence: an unavailable input makes the status incomplete and the primary code DEPENDENCY_UNAVAILABLE, before any other error. |
| IFCX-ERR-002 | error | BACKEND_UNAVAILABLE | A failing output sink or file system gives no output file. The file is written to a temporary name and installed only after complete success. |
| IFCX-ERR-003 | error | INVALID_ARGUMENT | An existing output file is not replaced unless overwrite is requested; a symbolic link or reparse point at the output name or its parent is refused. |
| IFCX-ERR-004 | error | CANCELLED | Cancellation before the install point discards all work and leaves no file. |
| IFCX-ERR-005 | mapping | - | In strict mode only an element-level finding that carries an F04 code (IFC_GEOMETRY_ABSENT and IFC_PLACEMENT_ABSENT, code UNSUPPORTED_GEOMETRY) refuses the export, with the code of the first such finding in finding order (F10 section 6.4). Warnings and info without a code (proxies, quantity omissions, truncated names, deactivated elements) are never fatal. |
| IFCX-EMIT-001 | mapping | - | An optional entity is written only when it has content, so no mandatory attribute or relationship is empty: IfcElementQuantity and its IfcRelDefinesByProperties only with at least one quantity; BeamCraft_Declared and its relationship only with a declared value; Pset_MaterialCommon only with a density; IfcMaterial and its IfcRelAssociatesMaterial only with at least one exported element using it; IfcRelContainedInSpatialStructure (of a storey or of the building) only with at least one exported element; the building-to-storeys IfcRelAggregates only with at least one storey; a representation only with a mesh. |
| IFCX-LIM-001 | error | LIMIT_EXCEEDED | At most 65,536 elements are exported in one file; more refuse the export before any output is produced. |
| IFCX-LIM-002 | error | LIMIT_EXCEEDED | The output is at most 268,435,456 bytes. The writer checks the running size before each write; exceeding it refuses the export and removes the temporary file. |
| IFCX-LIM-003 | mapping | - | Every count in the writer is bounded by a cap enforced upstream or by LIM-001, 002 and 005; no allocation is sized by an unchecked input, no recursion is used and no input-derived index is used. |
| IFCX-LIM-004 | mapping | - | The report keeps at most 256 findings in the F10 order and states total_findings and truncated; the per-element table is complete. |
| IFCX-LIM-005 | error | LIMIT_EXCEEDED | A single mesh of more than 262,144 vertices or 524,288 triangles, or a project of more than 1,024 levels, refuses the export. |
| IFCX-VAL-001 | mapping | - | Acceptance includes validating a written file with an independent IFC tool, run locally as a separate process with no upload of the file; it is a remaining acceptance check (section 19). |
| IFCX-SURF-001 | mapping | - | Every export is an engine API call first; the CLI command adds no computation and no private path (AGENTS.md section 3). |
| IFCX-SURF-002 | mapping | - | The export report of section 15.4 is deterministic, lists every element's GlobalId, representation, quantity freshness and omissions, and carries the SHA-256 of the file. |


## 17. Design choices and extension points

| Choice | Where |
|--------|-------|
| Target IFC4 (ADD2 TC1), not IFC 4.3; no model view is claimed | 2.1, 2.2, 8.2, 11.5, 18 |
| Geometry is the tessellated mesh only, in binary32 coordinates | 2.1, 6 |
| The ProjectId is stored in the project, never an export argument | 3.5, 4.1 |
| The header time stamp comes from the RevisionId | 11.5 |
| Every element is exported; a kind without an IFC mapping becomes `IfcBuildingElementProxy`; only deactivated elements are left out | 4.2, 13 |
| Stale, missing and unknown quantities are omitted and reported, never zero | 8 |
| The GlobalId is computed at export, never stored | 3.7 |
| Levels map to `IfcBuildingStorey` under a synthetic site and building | 5 |
| A material without a grade is named `Unspecified`, with its digest in a property | 7 |
| Declared section values are a labelled property set, never a quantity set | 9.2 |
| Non-element entities get GlobalIds from the SHA-256 construction | 3.5 |

**Extension point: IFCX.** A later version adds export to IFCX, the JSON-based successor format of
buildingSMART (IFC5), next to IFC4 STEP. The mapping of section 4 (which elements, in which order,
with which GlobalId, quantities and losses) is independent of the file format, so IFCX reuses it;
only the writer and the format-specific rules change. Version 0.1.0 writes IFC4 STEP only
(`IFCX-VER-002`). The rule prefix `IFCX-` of this contract means "IFC export" and is unrelated to
the format name.

**Extension point: export by selection.** A later version may export a user-defined filter or
selection of elements. Version 0.1.0 defines no option or behaviour for it; the element set is
chosen in one place (the pinned-snapshot element list of section 4.3), so a later version can
restrict it without changing the mapping rules.

## 18. Public specification index

All pages are from the public buildingSMART IFC specification, edition **IFC4 ADD2 TC1** (S01).
Section numbers are not quoted because the editions number them differently; the entity or
definition name identifies the page. URLs follow the pattern
`https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/<schema>/lexical/<lowercase name>.htm`
(quantity sets under `.../<schema>/qset/`, property sets under `.../<schema>/pset/`).
**Verification status:** the URLs below follow the edition's pattern. Rows marked "excerpt" were
checked against published excerpts, rows marked "not seen" are not yet checked against the page; a
row whose description proves wrong is a defect of this document, not of the specification.

| ID | Definition | URL | Used for | State |
|----|------------|-----|----------|-------|
| S01 | IFC4 ADD2 TC1 edition | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/> | Schema identifier IFC4 and edition | not seen |
| S02 | IFC-SPF, STEP physical file format | <https://technical.buildingsmart.org/standards/ifc/ifc-formats/ifc-spf/> | File structure, header, string escapes (the underlying ISO 10303-21 text is not public and is not used) | not seen |
| S03 | IfcGloballyUniqueId | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcutilityresource/lexical/ifcgloballyuniqueid.htm> | 22 characters; alphabet 0-9 A-Z a-z _ $; first byte fills the first two characters, further bytes in groups of three give four characters; first character 0 to 3 | excerpt (IFC4 FINAL page, section 8.21.2.1 there) |
| S04 | IfcLabel | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifclabel.htm> | Name type, limit of 255 characters | not seen |
| S05 | IfcIdentifier | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifcidentifier.htm> | Identifier property values | not seen |
| S06 | IfcText | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifctext.htm> | Unbounded text property values | not seen |
| S07 | IfcMassMeasure (and IfcInteger, IfcReal) | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifcmassmeasure.htm> | Mass and numeric property values | not seen |
| S10 | IfcProject | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifckernel/lexical/ifcproject.htm> | Unit assignment and representation contexts | not seen |
| S11 | IfcSite | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcsite.htm> | Synthetic site | not seen |
| S12 | IfcBuilding | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcbuilding.htm> | Synthetic building | not seen |
| S13 | IfcBuildingStorey | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcbuildingstorey.htm> | Storey: Name, Elevation, CompositionType, placement | not seen |
| S14 | IfcRelAggregates | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifckernel/lexical/ifcrelaggregates.htm> | Project, site, building, storeys decomposition | not seen |
| S15 | IfcRelContainedInSpatialStructure | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcrelcontainedinspatialstructure.htm> | Elements in a storey or the building | not seen |
| S16 | IfcBeam | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifcbeam.htm> | Beam entity, Body representation, predefined type | not seen |
| S17 | IfcColumn | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifccolumn.htm> | Column entity | not seen |
| S18 | IfcPlate | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifcplate.htm> | Plate entity | not seen (4.3 page listed in search) |
| S19 | IfcBuildingElementProxy | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcbuildingelementproxy.htm> | Element with no specific entity | not seen |
| S20 | IfcLocalPlacement | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometricconstraintresource/lexical/ifclocalplacement.htm> | Relative object placement | not seen |
| S21 | IfcAxis2Placement3D | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometryresource/lexical/ifcaxis2placement3d.htm> | Location, Axis (Z), RefDirection (X) | not seen |
| S22 | IfcCartesianPoint | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometryresource/lexical/ifccartesianpoint.htm> | Points | not seen |
| S23 | IfcDirection | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometryresource/lexical/ifcdirection.htm> | Directions | not seen |
| S24 | IfcProductDefinitionShape | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcrepresentationresource/lexical/ifcproductdefinitionshape.htm> | Shape of a product | not seen |
| S25 | IfcShapeRepresentation | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcrepresentationresource/lexical/ifcshaperepresentation.htm> | Identifier Body, type Tessellation | not seen |
| S26 | IfcGeometricRepresentationContext | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcrepresentationresource/lexical/ifcgeometricrepresentationcontext.htm> | Model context, precision | not seen |
| S27 | IfcGeometricRepresentationSubContext | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcrepresentationresource/lexical/ifcgeometricrepresentationsubcontext.htm> | Body sub-context | not seen |
| S28 | IfcTriangulatedFaceSet | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometricmodelresource/lexical/ifctriangulatedfaceset.htm> | Coordinates, optional Normals, optional Closed, CoordIndex of 1-based triples, optional PnIndex | not seen |
| S29 | IfcCartesianPointList3D | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcgeometryresource/lexical/ifccartesianpointlist3d.htm> | Coordinate list | not seen |
| S30 | IfcMaterial | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmaterialresource/lexical/ifcmaterial.htm> | Name (required), Description, Category | excerpt (IFC 4.3 page) |
| S31 | IfcRelAssociatesMaterial | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifckernel/lexical/ifcrelassociatesmaterial.htm> | Material to element association | excerpt (mentioned on the IfcMaterial page) |
| S32 | IfcMaterialProperties | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmaterialresource/lexical/ifcmaterialproperties.htm> | Named property group linked to a material | excerpt (IFC 4.3 page) |
| S33 | IfcPropertySet | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifckernel/lexical/ifcpropertyset.htm> | Custom property set | not seen |
| S34 | IfcPropertySingleValue | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcpropertyresource/lexical/ifcpropertysinglevalue.htm> | Single typed value with name and description | not seen |
| S35 | IfcRelDefinesByProperties | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifckernel/lexical/ifcreldefinesbyproperties.htm> | Property and quantity set to element | not seen |
| S36 | IfcElementQuantity | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcelementquantity.htm> | Quantity set | not seen |
| S37 | IfcQuantityVolume | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcquantityresource/lexical/ifcquantityvolume.htm> | Volume quantity | not seen |
| S38 | IfcQuantityWeight | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcquantityresource/lexical/ifcquantityweight.htm> | Weight quantity | not seen |
| S39 | IfcUnitAssignment | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifcunitassignment.htm> | Project units | not seen |
| S40 | IfcSIUnit | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifcsiunit.htm> | SI units with prefix | not seen |
| S41 | IfcDerivedUnit and IfcDerivedUnitElement | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmeasureresource/lexical/ifcderivedunit.htm> | Mass density unit | not seen |
| S42 | Qto_BeamBaseQuantities | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/qset/qto_beambasequantities.htm> | Length, CrossSectionArea, surface areas, GrossVolume, NetVolume, GrossWeight, NetWeight | excerpt (IFC4 FINAL page listed) |
| S43 | Qto_ColumnBaseQuantities | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/qset/qto_columnbasequantities.htm> | Column base quantities, assumed to mirror the beam set | not seen |
| S44 | Qto_PlateBaseQuantities | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/qset/qto_platebasequantities.htm> | Width, Perimeter, GrossArea, NetArea, GrossVolume, NetVolume; weights appear only in the 4.3 edition; net quantities account for openings by subtraction | excerpt (IFC4 FINAL page listed) |
| S45 | IfcBeamTypeEnum | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifcbeamtypeenum.htm> | Value BEAM | not seen |
| S46 | IfcColumnTypeEnum | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifccolumntypeenum.htm> | Value COLUMN | not seen |
| S47 | IfcPlateTypeEnum | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcsharedbldgelements/lexical/ifcplatetypeenum.htm> | IFC4 values: curtain panel, sheet, user defined, not defined | excerpt (a third-party enumeration listing, to verify) |
| S48 | IfcBuildingElementProxyTypeEnum | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcproductextension/lexical/ifcbuildingelementproxytypeenum.htm> | Value NOTDEFINED | not seen |
| S49 | Pset_MaterialCommon and IfcMassDensityMeasure | <https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/HTML/schema/ifcmaterialresource/pset/pset_materialcommon.htm> | MassDensity property of a material | not seen |

## 19. Remaining acceptance checks

Acceptance of the exporter still needs: (a) an independent check of a written file
with an IFC tool that runs locally as a separate process, reads the file without uploading it, has a
licence that allows that use, and validates against the IFC4 schema with the checks the cases need
(`IFCX-VAL-001`); (b) the vectors of section 3.2, the
synthetic vectors reproduced with the in-repo SHA-256, and the REAL and string vectors; (c) a check
of the written file against the published IFC4 Reference View definition, after which a later version
may name the view (section 2.1); (d) confirmation of the escapes of 11.4 against the independent
reader; (e) every reachable case of the case file.
