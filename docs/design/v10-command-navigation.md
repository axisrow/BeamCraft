# V10 command navigation

An experimental UI shell: command categories and a workspace drawer. This is not completion of
the V10 acceptance gate or an implementation of the commands shown as placeholders.

The tool row has a three-horizontal-line menu button immediately left of Select, outside the
horizontal tool scroller. It opens an animated docked panel along the left edge below the ribbon, with
groups for model files, exchange/output, project preferences, support and the session. All its
command entries are disabled, including file and session entries. They dispatch no commands.
The same toggle, the close button and Esc collapse the panel. It shares the window layout with
the viewport, with no floating frame, shadow or backdrop. Other controls and the model stay
interactive while it is open. Opening it changes the viewport's available width, never model data.
The menu icon is painted from three horizontal strokes rather than a Unicode character, so the
font cannot substitute a word or a different symbol. Its accessible name and tooltip remain text.
The menu occupies a narrow, full-height rail flush with the window's left edge. Selection sits
next to it with the original rounded active face, outline and offset shadow. The icon centres
share a horizontal axis, independent of group captions or scrollbar height. Both controls stay
outside the tool scroller.

Five ribbon tabs organize commands: Model, Modify, View, Drawings and Project. Tab navigation is
active, using text tabs with an active underline and no button frames. Selection uses a single
pointer icon and a tooltip; there is no repeated Select caption. Existing create/edit tools keep
their CLI paths; the additional modelling, transformation,
view, drawing and project buttons are disabled and explain their availability on hover. Tabs and
tools scroll horizontally when the window is narrow. Navigation changes no model data.

The viewport has no floating Fit, plus/minus or 3D/Plan/Grid A controls. Wheel and keyboard
navigation still operate through the existing camera. Orbit dragging moves the near model
surface in the pointer's direction, for both the mouse and trackpad presets.

The layout, labels and grouping are contributor-original BeamCraft code. Appearance uses the
existing BeamCraft theme, Inter font and procedural icons. No new asset or dependency is added;
no third-party screenshot, icon, palette or other asset is included or traced.

Verification uses the real egui input/layout path: the menu button is on the tool row left of
Select; opening the panel reserves width beside the viewport; an inert file command keeps it
open; other ribbon tabs still work; the toggle and Esc collapse it. These actions preserve the
snapshot and start no edit. No modal layer is created.
The drawer test checks that its command widgets are disabled. Existing UI-to-CLI end-to-end tests
continue to cover the implemented tools.
