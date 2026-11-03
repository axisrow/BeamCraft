# U08 portable appearance

Every classed part carries a display colour that any USD consumer can see, without running
BeamCraft. The colour comes from presentation — the part's `steel:class` read through the P15
class palette (`beamcraft_model::class_palette`) — never from the engineering material: a
steel grade colours nothing.

## The authored form

A create or a class edit of a plate, beam, column or instance writes three opinions beside
`steel:class`, replaced as a whole on every class edit:

```usda
def Xform "PRT_…" (
    prepend apiSchemas = ["MaterialBindingAPI"]
)
{
    custom int steel:class = 2
    custom uniform color3f[] primvars:displayColor = [(r, g, b)]
    custom rel material:binding = </World/Looks/PartClass2>
}
```

`primvars:displayColor` is the fallback: viewers without material support still colour by it,
and it inherits to the derived `Geometry` mesh. `material:binding` is the portable material:
the reserved `/World/Looks` scope holds the eight fixed looks `PartClass0` … `PartClass7`,
one `UsdShade` Material per palette entry whose `outputs:surface` connects to a
`UsdPreviewSurface` shader child with the entry's `inputs:diffuseColor`. The look a class
binds to is its palette index (`unsigned |class| mod 8`, the same wrap-around the engine's
`class_colour` applies), so the viewport overlay (V17) and the authored layers agree.

The palette bytes are sRGB display data; the authored USD values are their linear RGB, so
external viewers show the same colours the app does. The looks are authored with the first
classed part and never removed: they are fixed content, bounded (eight materials, one shader
each), and validated byte-exact by the strict profile — no other owned prim may live under
`/World/Looks`, and a tampered colour, binding or stray look refuses the layer.

## What does not change

The engineering material (`bim:material`), the profile and every quantity are untouched; a
material edit changes nothing about the appearance. Non-part kinds (assembly, connection,
level, grid) carry no appearance; a binding or display colour on one is refused. The V17
colour-set overlay stays per-window display state and is never stored.
