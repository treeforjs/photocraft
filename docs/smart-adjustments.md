# Editable adjustments on Smart Objects

PhotoCraft supports a complete native editing workflow for **Levels, Curves and
Shadows/Highlights** on Smart Objects.

1. Select an image layer and choose **Filter › Convert for Smart Filters**.
2. Choose one of those adjustments under **Image › Adjustments**. Preview the
   settings and press OK to add a Smart Filter without rewriting its source pixels.
3. Double-click the adjustment under **Smart Filters** in the Layers panel to edit
   it again. Its stored settings, including individual color-channel curves and
   levels, populate the dialog. Preview replaces that filter in the existing stack.
4. Press OK for one undoable edit, or Cancel to leave the document unchanged.
   Save as `.pcraft` to retain the complete stack and its source.

A selection becomes the stack's filter mask when there is no existing filter mask.
Subsequent adjustments preserve that mask. Targeting a layer mask, alpha channel
or Quick Mask continues to edit that target rather than adding a Smart Filter.
Select the composite color channel when adding a Smart Filter; single color-channel
targets are not supported for this workflow. Pixel locks, including inherited group
locks, prevent edits.

CLI, control and MCP clients use the same `image.adjustments.levels`,
`image.adjustments.curves`, `image.adjustments.shadowsHighlights` and
`layer.smartFilter.setParams` commands. The control channel can reopen the dialog
with `ui.dialog.open {"kind":"smartFilter","layer":42,"index":0}`; see
[the protocol](control-protocol.md).

## Current limits

- RGB Levels and Curves, and default Shadows/Highlights, have live PSD descriptor
  mappings. Custom Shadows/Highlights and CMYK/Lab channel settings produce an
  export warning because their live descriptors are unsupported. Keep the `.pcraft`
  original for editable work. Generated PSDs are tested with PhotoCraft's strict
  parser and importer; third-party acceptance is not established by those tests.
- Hue/Saturation and Colorize on Smart Objects remain unimplemented.
- Enabled stacks containing visible unsupported filters in imported files block adjustment
  creation and parameter editing. Other existing stack actions retain their behavior. Hiding that
  filter explicitly permits a new rendering; the unknown descriptor remains stored.
- Missing linked sources must be relinked before editing.
- Reduced previews are supported for stacks containing these three adjustments.
  Other filter kinds, layer effects or enabled vector masks retain full-resolution previews. Reduced previews
  approximate sampling and neighborhood effects; committed processing is full size.
- Curves eyedroppers use the existing committed merged-image sampler. They do not
  sample the isolated pixels below the selected Smart Filter.
- This work does not change filter mathematics, corpus fidelity floors or the
  overall parity estimate.

The synthetic acceptance suite lives in `crates/engine/tests/smart_adjustments.rs`;
dialog and scaled-placement regressions are in
`crates/ui-egui/src/smart_adjust_preview.rs`.
