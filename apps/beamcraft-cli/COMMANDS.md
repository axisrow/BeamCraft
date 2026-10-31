# BeamCraft CLI command reference

Generated from `src/registry.rs`; do not edit. Regenerate it with
`BEAMCRAFT_BLESS=1 cargo test -p beamcraft-cli --test cli registry::the_reference_is_current`.

A command is `beamcraft COMMAND --option VALUE ...`. In a usage line an option written bare is
required, `[--option VALUE]` is optional, and `(--a X | --b Y)` means exactly one of them is
required. A rule that links optional options (for example `--width` with `--length`) is in
the family help, `beamcraft --help`, and in the refusal. `read` commands change nothing
persistent; `write` commands change the store, the settings, a library or an output file.

## blobs

### put-blob

write.

```text
put-blob --store ROOT --file FILE [--timeout-ms U64] [--cancel-after-ms U64]
```

### verify-blob

read.

```text
verify-blob --store ROOT --digest SHA256 [--timeout-ms U64] [--cancel-after-ms U64]
```

## revision

### commit-revision

write.

```text
commit-revision --store ROOT --project-id UUID --workspace-id UUID --revision-id UUID --op-id UUID
    --generation U64 --base none|UUID --schema SHA256 --root-layer TEXT
    --layers ROLE:NAME:SHA256,... [--timeout-ms U64] [--cancel-after-ms U64]
```

### open-revision

read.

```text
open-revision --store ROOT [--timeout-ms U64] [--cancel-after-ms U64]
```

### recover-revision

write.

```text
recover-revision --store ROOT --min-age-secs U64 [--timeout-ms U64] [--cancel-after-ms U64]
```

### inspect-revision-outcome

read.

```text
inspect-revision-outcome --store ROOT --op-id UUID [--timeout-ms U64] [--cancel-after-ms U64]
```

## plate

### create-plate

write.

```text
create-plate --store ROOT --thickness MM [--width MM] [--length MM]
    [--contour X,Y[,Z];X,Y[,Z];...] [--normalize-winding true|false]
    [--thickness-side top|middle|bottom] [--corners V=chamfer:D1[,D2];V=round:R;...] [--name TEXT]
    [--class INT32] [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--translation X,Y,Z]
    [--orientation-xyzw X,Y,Z,W] [--origin-at-grid GRID:LABEL,LABEL] [--z MM] [--level UUID]
    [--offset MM] [--material-digest SHA256] [--material TEXT] [--hole-rect X,Y,W,H;X,Y,W,H;...]
    [--hole-round X,Y,D;X,Y,D;...] [--hole-segments N] [--new-id UUID] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### create-lofted-plate

write.

```text
create-lofted-plate --store ROOT --start X,Y,Z;X,Y,Z;... --end X,Y,Z;X,Y,Z;... [--name TEXT]
    [--class INT32] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--material-digest SHA256]
    [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### inspect-lofted-plates

read.

```text
inspect-lofted-plates --store ROOT
```

### add-hole

write.

```text
add-hole --store ROOT --id UUID (--polygon X,Y;X,Y;... | --hole-rect X,Y,W,H | --hole-round X,Y,D)
    [--axis-point X,Y,Z] [--axis-direction X,Y,Z] [--diameter MM] [--from MM] [--to MM]
    [--operation-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### update-hole

write.

```text
update-hole --store ROOT --id UUID --operation-id UUID
    (--polygon X,Y;X,Y;... | --hole-rect X,Y,W,H | --hole-round X,Y,D) [--axis-point X,Y,Z]
    [--axis-direction X,Y,Z] [--diameter MM] [--from MM] [--to MM] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### remove-hole

write.

```text
remove-hole --store ROOT --id UUID --operation-id UUID [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### inspect-elements

read.

```text
inspect-elements --store ROOT [--element UUID]
```

## definitions

### define-material

write.

```text
define-material --store ROOT [--density N] [--grade TEXT] [--standard TEXT] [--name TEXT]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### define-profile

write.

```text
define-profile --store ROOT [--width MM] [--height MM]
    [--family i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega|custom] [--h MM]
    [--b MM] [--tw MM] [--tf MM] [--t MM] [--r MM] [--c MM] [--d MM] [--outer X,Y;X,Y;...]
    [--cavity X,Y;...|X,Y;...] [--library FILE] [--environment FILE] [--user-library FILE]
    [--designation TEXT] [--standard TEXT] [--area-mm2 N] [--mass-per-meter-kg N]
    [--surface-per-meter-m2 N] [--name TEXT] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### define-fastener

write.

```text
define-fastener --store ROOT --fastener-library FILE --designation TEXT [--name TEXT]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### definitions

read.

```text
definitions --store ROOT [--name TEXT] [--offset N] [--limit N]
```

## library

### list-library

read.

```text
list-library (--library FILE | --environment FILE | --fastener-library FILE) [--user-library FILE]
    [--family TEXT] [--filter TEXT] [--offset N] [--limit N] [--shadow-offset N]
```

### show-library

read.

```text
show-library --designation TEXT (--library FILE | --environment FILE | --fastener-library FILE)
    [--user-library FILE]
```

### add-library

write.

```text
add-library --library FILE --designation TEXT [--library-id TEXT] [--title TEXT] [--width MM]
    [--height MM] [--family i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega|custom]
    [--h MM] [--b MM] [--tw MM] [--tf MM] [--t MM] [--r MM] [--c MM] [--d MM]
    [--outer X,Y;X,Y;...] [--cavity X,Y;X,Y;...|X,Y;X,Y;...] [--standard TEXT] [--area-mm2 N]
    [--mass-per-meter-kg N] [--surface-per-meter-m2 N]
```

## member

### create-member

write.

```text
create-member --store ROOT (--profile-digest SHA256 | --profile TEXT)
    (--start X,Y,Z | --start-at-grid GRID:LABEL,LABEL)
    (--end X,Y,Z | --end-at-grid GRID:LABEL,LABEL) [--kind beam|column] [--axis-lock none|x|y|z]
    [--roll RAD] [--material-digest SHA256] [--material TEXT] [--name TEXT] [--class INT32]
    [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W]
    [--z MM] [--level UUID] [--offset MM] [--section-plane left|centre|right]
    [--section-plane-offset MM] [--section-depth front|centre|back] [--section-depth-offset MM]
    [--start-offset MM] [--end-offset MM] [--new-id UUID] [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### create-polybeam

write.

```text
create-polybeam --store ROOT --points X,Y,Z;X,Y,Z;... (--profile-digest SHA256 | --profile TEXT)
    [--kind beam|column] [--material-digest SHA256] [--material TEXT] [--name TEXT]
    [--class INT32] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--new-id UUID]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### create-curved-beam

write.

```text
create-curved-beam --store ROOT (--profile-digest SHA256 | --profile TEXT)
    [--points X,Y,Z;X,Y,Z;...] [--start X,Y,Z] [--end X,Y,Z] [--radius MM] [--normal X,Y,Z]
    [--kind beam|column] [--material-digest SHA256] [--material TEXT] [--name TEXT]
    [--class INT32] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W] [--new-id UUID]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

## work_plane

### work-plane

read.

```text
work-plane
    (--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ | --parallel xy|xz|yz | --points OX,OY,OZ,PX,PY,PZ,QX,QY,QZ | --vertical AX,AY,AZ,BX,BY,BZ | --store ROOT)
    [--offset MM] [--origin X,Y,Z] [--direction X,Y,Z]
```

### resolve-construction-point

read.

```text
resolve-construction-point --reference X,Y,Z (--toward X,Y,Z | --origin X,Y,Z) [--direction X,Y,Z]
    [--distance MM] [--ortho true|false] [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ]
```

## edit

### update-element

write.

```text
update-element --store ROOT --id UUID [--name TEXT] [--class INT32] [--profile-digest SHA256]
    [--profile TEXT] [--material-digest SHA256|none] [--thickness MM]
    [--thickness-side top|middle|bottom] [--corners V=chamfer:D1[,D2];V=round:R|none]
    [--axis-point X,Y,Z;X,Y,Z;...] [--translation X,Y,Z] [--orientation-xyzw X,Y,Z,W|none]
    [--level UUID|none] [--rotate-about X,Y,Z] [--rotate-axis X,Y,Z] [--rotate-angle RAD]
    [--mirror-point X,Y,Z] [--mirror-normal X,Y,Z] [--move-from X,Y,Z] [--move-to X,Y,Z]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### move-plate-vertex

write.

```text
move-plate-vertex --store ROOT --id UUID --vertex N --to X,Y [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### move-plate-edge

write.

```text
move-plate-edge --store ROOT --id UUID --edge N --delta DX,DY [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### insert-plate-vertex

write.

```text
insert-plate-vertex --store ROOT --id UUID --edge N --point X,Y [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### delete-plate-vertex

write.

```text
delete-plate-vertex --store ROOT --id UUID --vertex N [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### set-plate-corner

write.

```text
set-plate-corner --store ROOT --id UUID --vertex N --corner chamfer:D1[,D2]|round:R|sharp
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### cut-by-plane

write.

```text
cut-by-plane --store ROOT --id UUID --operation-id UUID --point X,Y,Z --normal X,Y,Z
    --keep positive|negative [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### update-cut

write.

```text
update-cut --store ROOT --id UUID --operation-id UUID --point X,Y,Z --normal X,Y,Z
    --keep positive|negative [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### remove-cut

write.

```text
remove-cut --store ROOT --id UUID --operation-id UUID [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### cut-by-line

write.

```text
cut-by-line --store ROOT --id UUID --operation-id UUID --point-a X,Y,Z --point-b X,Y,Z
    --axis X,Y,Z --keep positive|negative [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### cut-by-polygon

write.

```text
cut-by-polygon --store ROOT --id UUID --operation-id UUID --polygon X,Y;X,Y;... [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### cut-by-part

write.

```text
cut-by-part --store ROOT --id UUID --operation-id UUID --tool UUID [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### fit-end-to-part

write.

```text
fit-end-to-part --store ROOT --id UUID --operation-id UUID --tool UUID [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### chamfer-part

write.

```text
chamfer-part --store ROOT --id UUID --operation-id UUID --vertex-index N --first-distance MM
    --second-distance MM [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### split-part

write.

```text
split-part --store ROOT --id UUID --new-id UUID --new-name TEXT --operation-id UUID --point X,Y,Z
    --normal X,Y,Z --keep positive|negative [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### combine-parts

write.

```text
combine-parts --store ROOT --survivor UUID --consumed UUID --operation-id UUID [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### bend-plate

write.

```text
bend-plate --store ROOT --id UUID --operation-id UUID --kind fold|roll --point-a X,Y,Z
    --point-b X,Y,Z --radius MM [--radius-b MM] [--angle-rad RAD] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

## delete

### delete-element

write.

```text
delete-element --store ROOT --id UUID [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

## copy

### copy-element

write.

```text
copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--copy-from X,Y,Z]
    [--copy-to X,Y,Z] [--new-id UUID] [--new-axis-ids UUID,UUID,...] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### copy-instance

write.

```text
copy-instance --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id UUID] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### array-linear

write.

```text
array-linear --store ROOT --id UUID --count N --direction X,Y,Z --spacing MM [--name TEXT]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### array-polar

write.

```text
array-polar --store ROOT --id UUID --count N --point X,Y,Z --axis X,Y,Z --angle RAD [--name TEXT]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

## workspace

### create-workspace

write.

```text
create-workspace --store ROOT --workspace-dir DIR [--workspace-id UUID] [--user-id UUID]
    [--revision-id UUID] [--op-id UUID]
```

### inspect-workspace

read.

```text
inspect-workspace --store ROOT --workspace-dir DIR
```

### publish-workspace

write.

```text
publish-workspace --store ROOT --workspace-dir DIR
```

### resolve-workspace

write.

```text
resolve-workspace --workspace-dir DIR --op-id UUID --discard UUID --expected-draft UUID
    [--actor-id UUID]
```

### undo-workspace

write.

```text
undo-workspace --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
```

### redo-workspace

write.

```text
redo-workspace --workspace-dir DIR [--revision-id UUID] [--op-id UUID]
```

### inspect-workspace-history

read.

```text
inspect-workspace-history --workspace-dir DIR
```

## rebase

### rebase-workspace

write.

```text
rebase-workspace --store ROOT --workspace-dir DIR --op-id UUID --expected-head UUID
    --expected-draft UUID [--actor-id UUID]
```

### compensate-workspace

write.

```text
compensate-workspace --store ROOT --workspace-dir DIR --target-revision UUID --op-id UUID
    --expected-head UUID [--actor-id UUID]
```

## reference

### attach-reference

write.

```text
attach-reference --store ROOT --source FILE [--name TEXT] [--translation X,Y,Z]
    [--rotation-z-deg DEG] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### move-reference

write.

```text
move-reference --store ROOT --id UUID [--translation X,Y,Z] [--rotation-z-deg DEG] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### correct-reference-units

write.

```text
correct-reference-units --store ROOT --id UUID [--meters-per-unit F] [--up-axis Y|Z]
    [--clear true] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### write-reference

read.

```text
write-reference --store ROOT --id UUID (--target source | --prim /DefaultPrim/Path)
    [--attribute NAME]
```

### inspect-reference

read.

```text
inspect-reference --store ROOT
```

### measure-reference

read.

```text
measure-reference --store ROOT [--id UUID]
```

## assembly

### create-assembly

write.

```text
create-assembly --store ROOT --name TEXT [--new-id UUID] [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### attach-to-assembly

write.

```text
attach-to-assembly --store ROOT --id UUID --assembly-id UUID [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### detach-from-assembly

write.

```text
detach-from-assembly --store ROOT --id UUID [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### set-main-part

write.

```text
set-main-part --store ROOT --id UUID --assembly-id UUID [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### explode-assembly

write.

```text
explode-assembly --store ROOT --id UUID [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### assemblies

read.

```text
assemblies --store ROOT
```

## weld

### create-weld

write.

```text
create-weld --store ROOT --name TEXT --main UUID --secondary UUID --size MM
    --location workshop|site --side arrow|other|both [--type fillet] [--path X,Y,Z;X,Y,Z;...]
    [--all-around true|false] [--contour none|flush|concave|convex] [--new-id UUID] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### welds

read.

```text
welds --store ROOT
```

## bolt

### create-bolt

write.

```text
create-bolt --store ROOT --name TEXT --main UUID --secondary UUID --origin X,Y,Z --direction X,Y,Z
    --count N --pitch MM --diameter MM [--new-id UUID] [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### bolts

read.

```text
bolts --store ROOT
```

### fastener-geometry

read.

```text
fastener-geometry --fastener-library FILE --designation TEXT --length MM [--count N]
    [--density KG_M3] [--washer-under-head true|false] [--washer-under-nut true|false]
```

## level

### create-level

write.

```text
create-level --store ROOT --name TEXT --elevation MM [--new-id UUID] [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### levels

read.

```text
levels --store ROOT
```

## attribute

### attribute-define

write.

```text
attribute-define --store ROOT --name TEXT --type text|number|integer|boolean|choice [--unit TEXT]
    [--options A,B,C] [--set TEXT] [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### attribute-undefine

write.

```text
attribute-undefine --store ROOT --name TEXT [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### attribute-set

write.

```text
attribute-set --store ROOT --element UUID --name TEXT --value TEXT [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### attribute-clear

write.

```text
attribute-clear --store ROOT --element UUID --name TEXT [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### attribute-get

read.

```text
attribute-get --store ROOT --element UUID
```

### attributes

read.

```text
attributes --store ROOT
```

## filters

### query-elements

read.

```text
query-elements --store ROOT --rule FILE [--limit N] [--after UUID] [--expected-model SHA256|none]
```

## grid

### create-grid

write.

```text
create-grid --store ROOT --name TEXT [--new-id UUID] [--origin X,Y] [--rotation DEG]
    [--u-expression TEXT] [--u-labels TEXT] [--u-scheme numeric|alpha] [--u-label-start TEXT]
    [--u-label-step N] [--u-label-order ascending|descending] [--u-axis-ids UUID,UUID,...]
    [--u-span MM,MM] [--u-from X,Y] [--u-to X,Y] [--u-label TEXT] [--u-axis-id UUID]
    [--v-expression TEXT] [--v-labels TEXT] [--v-scheme numeric|alpha] [--v-label-start TEXT]
    [--v-label-step N] [--v-label-order ascending|descending] [--v-axis-ids UUID,UUID,...]
    [--v-span MM,MM] [--v-from X,Y] [--v-to X,Y] [--v-label TEXT] [--v-axis-id UUID]
    [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### update-grid

write.

```text
update-grid --store ROOT --id UUID [--name TEXT] [--origin X,Y] [--rotation DEG]
    [--fit-spans true|false] [--u-expression TEXT] [--u-labels TEXT]
    [--u-new-axis-ids UUID,UUID,...] [--u-span MM,MM] [--v-expression TEXT] [--v-labels TEXT]
    [--v-new-axis-ids UUID,UUID,...] [--v-span MM,MM] [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### list-grids

read.

```text
list-grids --store ROOT
```

### resolve-grid-point

read.

```text
resolve-grid-point --store ROOT --grid UUID|TEXT --labels U,V
```

### resolve-grid-plane

read.

```text
resolve-grid-plane --store ROOT --grid UUID|TEXT --label TEXT
```

### draw-grid-line

write.

```text
draw-grid-line --store ROOT --id UUID --family u|v --label TEXT --from X,Y --to X,Y
    [--new-axis-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### reshape-grid-line

write.

```text
reshape-grid-line --store ROOT --id UUID --axis-id UUID --from X,Y --to X,Y [--op-id UUID]
    [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### remove-grid-line

write.

```text
remove-grid-line --store ROOT --id UUID --axis-id UUID [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### relabel-grid-line

write.

```text
relabel-grid-line --store ROOT --id UUID [--axis-id UUID] [--label TEXT] [--family u|v]
    [--labels TEXT] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

## construction

### create-construction

write.

```text
create-construction --store ROOT --name TEXT --kind point|line|circle|plane --a X,Y,Z [--b X,Y,Z]
    [--radius MM] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID]
    [--project-id UUID] [--workspace-id UUID]
```

### list-construction

read.

```text
list-construction --store ROOT
```

## export

### export-ifc

write.

```text
export-ifc --store ROOT --out FILE [--revision UUID] [--project-name TEXT] [--schema IFC4]
    [--format step] [--report FILE] [--strict] [--overwrite]
```

### ifc-guid

read.

```text
ifc-guid (--id UUID | --decode GLOBALID)
```

## numbering

### preview-numbering

read.

```text
preview-numbering --store ROOT [--plate-series PREFIX:START] [--beam-series PREFIX:START]
    [--column-series PREFIX:START] [--assembly-series PREFIX:START] [--tolerance MM]
    [--compare-name true|false] [--previous SHA256] [--offset N] [--limit N]
```

### approve-numbering

write.

```text
approve-numbering --store ROOT --preview-token SHA256 [--plate-series PREFIX:START]
    [--beam-series PREFIX:START] [--column-series PREFIX:START] [--assembly-series PREFIX:START]
    [--tolerance MM] [--compare-name true|false] [--op-id UUID] [--revision-id UUID]
    [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]
```

### inspect-numbering

read.

```text
inspect-numbering --store ROOT [--offset N] [--limit N]
```

## changeset

### create-changeset

write.

```text
create-changeset --store ROOT --workspace-dir DIR [--changeset-id UUID] [--revision-id UUID]
    [--actor-id UUID]
```

### inspect-changeset

read.

```text
inspect-changeset --workspace-dir DIR --changeset SHA256
```

### apply-changeset

write.

```text
apply-changeset --store ROOT --workspace-dir DIR --changeset SHA256 --actor-id UUID
```

## coordinator

### serve-coordinator

write.

```text
serve-coordinator --store ROOT --workspaces-root DIR --client ACTOR_UUID:TOKEN_FILE [--port N]
    [--max-requests N]
```

### publish-changeset

write.

```text
publish-changeset --address IP:PORT --token-file FILE --workspace NAME --changeset SHA256
    --expected-head UUID
```

### coordinator-compensate

write.

```text
coordinator-compensate --address IP:PORT --token-file FILE --workspace NAME --op-id UUID
    --target-revision UUID --expected-head UUID
```

### coordinator-rebase

write.

```text
coordinator-rebase --address IP:PORT --token-file FILE --workspace NAME --op-id UUID
    --expected-head UUID --expected-draft UUID
```

### inspect-coordinator-head

read.

```text
inspect-coordinator-head --address IP:PORT --token-file FILE
```

## views

### list-views

read.

```text
list-views --file FILE [--store ROOT]
```

### save-view

write.

```text
save-view --file FILE --name TEXT [--store ROOT] [--target X,Y,Z] [--yaw RAD] [--pitch RAD]
    [--distance MM] [--ortho true|false] [--work-area MM,MM,MM,MM,MM,MM] [--plane-grid SELECTOR]
    [--plane-line LABEL] [--plane-reverse true|false] [--plane-level true|false] [--from X,Y,Z]
    [--to X,Y,Z] [--centre X,Y,Z]
```

### remove-view

write.

```text
remove-view --file FILE --name TEXT
```

### apply-view

read.

```text
apply-view --file FILE --name TEXT
```

## history

### inspect-history

read.

```text
inspect-history --store ROOT [--workspaces-root DIR] [--workspace-dir DIR]
```

### compact-history

write.

```text
compact-history --store ROOT (--keep-head | --keep N) [--dry-run] [--yes] [--backup DIR]
    [--no-backup] [--workspaces-root DIR] [--workspace-dir DIR] [--no-workspaces]
    [--ledger-tail N] [--orphan-grace-secs N]
```

## view

### snapshot-view

read.

```text
snapshot-view --plates N [--hide KIND,...] [--representation full|simplified]
    [--stand-in-simplified true|false] [--select UUID,...] [--revision SHA256] [--generation U64]
```

### inspect-view-changes

read.

```text
inspect-view-changes --plates N --edit move-first|resize-first|add-one|remove-last,...
    [--since TOKEN] [--stand-in-simplified true|false] [--revision SHA256] [--generation U64]
```

### clip-view

read.

```text
clip-view --plates N [--cut X,Y,Z,X,Y,Z;...] [--area MM,MM,MM,MM,MM,MM]
    [--stand-in-simplified true|false] [--revision SHA256] [--generation U64]
```

## pick

### pick

read.

```text
pick --store ROOT --targets face,edge,vertex [--origin X,Y,Z] [--direction X,Y,Z] [--t-max MM]
    [--include-backfaces true|false] [--point X,Y,Z] [--radius MM] [--max-hits N]
    [--exclude KIND,...]
```

### snap

read.

```text
snap --store ROOT --point X,Y,Z [--radius MM] [--kinds KIND,...] [--max-hits N]
    [--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ] [--origin X,Y,Z] [--direction X,Y,Z] [--aperture F]
    [--cursor-radius MM] [--reference-snaps true|false] [--geometry-snaps true|false]
    [--grid-snaps true|false] [--nearest true|false]
```

### lookup-feature

read.

```text
lookup-feature --store ROOT --feature-id TEXT
```

### list-feature

read.

```text
list-feature --store ROOT --id UUID
```

### element-edges

read.

```text
element-edges --store ROOT --id UUID
```

## view_export

### export-view

write.

```text
export-view --store ROOT --out FILE [--with-definitions true|false] [--with-element UUID]
    [--with-grids true|false]
```

## bom

### export-bom

write.

```text
export-bom --store ROOT --out FILE [--summary FILE]
```

## schedules

### schedule-export

write.

```text
schedule-export --store ROOT --template FILE --out FILE [--summary FILE] [--html FILE]
```

## drawing

### preview-drawing

write.

```text
preview-drawing --store ROOT --id UUID --drawing-id UUID --view-id UUID --dimension-id UUID
    --anchor-a FEATURE_ID --anchor-b FEATURE_ID [--name TEXT] [--plane front|top|right]
    [--scale N] [--origin X,Y] [--offset MM] [--decimals 0..255] [--intent-out FILE]
```

### create-drawing

write.

```text
create-drawing --store ROOT --document-dir DIR --op-id UUID --revision-id UUID --actor-id UUID
    --id UUID --drawing-id UUID --view-id UUID --dimension-id UUID --anchor-a FEATURE_ID
    --anchor-b FEATURE_ID [--name TEXT] [--plane front|top|right] [--scale N] [--origin X,Y]
    [--offset MM] [--decimals 0..255]
```

### refresh-drawing

write.

```text
refresh-drawing --store ROOT --document-dir DIR --expected-head UUID --op-id UUID
    --revision-id UUID --actor-id UUID --id UUID --drawing-id UUID --view-id UUID
    --dimension-id UUID --anchor-a FEATURE_ID --anchor-b FEATURE_ID [--name TEXT]
    [--plane front|top|right] [--scale N] [--origin X,Y] [--offset MM] [--decimals 0..255]
```

### edit-drafting

write.

```text
edit-drafting --document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID
    --actor-id UUID [--drafting UUID:kind:X,Y,...]
```

### edit-views

write.

```text
edit-views --document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID --actor-id UUID
    --placed-view SPEC
```

### publish-drawing

write.

```text
publish-drawing --document-dir DIR --expected-head UUID --op-id UUID --revision-id UUID
    --actor-id UUID --version-id UUID
```

### inspect-drawing

read.

```text
inspect-drawing --document-dir DIR [--model-store ROOT]
```

### inspect-drawing-versions

read.

```text
inspect-drawing-versions --document-dir DIR
```

### export-drawing

write.

```text
export-drawing --document-dir DIR --version-id UUID --out FILE [--format svg|pdf]
    [--sheet a4|a3|a2|a1|a0]
```

## start

### create-project

write.

```text
create-project --parent DIR --name TEXT [--unique-name true|false] [--config-dir DIR]
    [--environment ID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID]
    [--workspace-id UUID]
```

### list-project

read.

```text
list-project --dir DIR
```

## commands

### commands

read.

```text
commands
```
