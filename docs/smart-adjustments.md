# Editable adjustments on Smart Objects

PhotoCraft supports a native editing workflow for **Levels, Curves,
Hue/Saturation (including Colorize), and Shadows/Highlights** on Smart Objects.

1. Select an image layer and choose **Filter › Convert for Smart Filters**.
2. Choose one of those adjustments under **Image › Adjustments**. Preview the
   settings and press OK to add a Smart Filter without rewriting its source pixels.
3. Double-click the adjustment under **Smart Filters** in the Layers panel to edit
   it again. Its stored settings, including individual color-channel curves and
   levels and all six Hue/Saturation ranges, populate the dialog. Preview replaces
   that filter in the existing stack.
4. Press OK for one undoable edit, or Cancel to leave the document unchanged.
   Save as `.pcraft` to retain the complete stack and its source.

A selection becomes the stack's filter mask when there is no existing filter mask.
Subsequent adjustments preserve that mask. Targeting a layer mask, alpha channel
or Quick Mask continues to edit that target rather than adding a Smart Filter.
Select the composite color channel when adding a Smart Filter; single color-channel
targets are not supported for this workflow. Pixel locks, including inherited group
locks, prevent edits.

CLI, control and MCP clients use the same `image.adjustments.levels`,
`image.adjustments.curves`, `image.adjustments.hueSaturation`,
`image.adjustments.shadowsHighlights` and
`layer.smartFilter.setParams` commands. The control channel can reopen the dialog
with `ui.dialog.open {"kind":"smartFilter","layer":42,"index":0}`; see
[the protocol](control-protocol.md).

Parameter edits merge individual controls inside Levels channels and Hue/Saturation
ranges. For example, `{"params":{"reds":{"hue":25}}}` changes the red-range hue
without resetting its saturation, lightness or range boundaries. Passing a complete
record as `null` resets that record to its defaults. Colorize retains the six range
settings so they are available again when Colorize is switched off. Its saturation
accepts the full 0–100% range; 0% produces gray.

## Filter masks and stack controls

Click the grayscale thumbnail beside **Smart Filters** to target the stack's mask,
then use Brush, Pencil or Eraser. A stack without a stored mask starts with an
implicit white mask. The live stroke updates its rendered appearance; releasing
the pointer commits the mask and cache in one undo step. Escape cancels the stroke.
Changing the document, layer or channel while painting invalidates the gesture.
Shift-click the thumbnail to enable or disable the filter mask.

The stack disclosure arrow collapses its rows without editing the document. The
blending icon opens **Smart Filter Blending Options**, with replacement preview,
Cancel and one-step confirmation. Drag a filter above or below another row to
reorder it within the same layer. The context menu also provides edit, blending,
visibility, move and delete actions.

Automation can select the mask with `ui.set {"smartFilterMaskTarget":true}` or
send `paint.stroke` / `paint.pencil` with `"target":"smartFilterMask"` and an
optional `layer`. Eraser uses the brush command's `erase` option. The control
channel exposes collapse through `ui.set {"smartFiltersCollapsed":true}` and
blending through `ui.dialog.open {"kind":"smartFilterBlend","layer":42,"index":0}`.
Its `opacity` field is a fraction from 0 to 1; `blend` takes the existing blend-mode
names. Ordinary dialog set, confirm and cancel methods apply.

Painting prepares the source and filtered image once per stroke. If an imported
cache or externally changed linked source differs from a fresh render, the first
preview replaces the complete stale cache, including pixels outside the canvas.
Subsequent dabs update their damaged regions. Cancelling still restores the original
document. Floating-point HDR samples and unfiltered source pixels are preserved.

## Current limits

- RGB Levels and Curves, and default Shadows/Highlights, have live PSD descriptor
  mappings. Hue/Saturation (including Colorize), custom Shadows/Highlights and
  CMYK/Lab channel settings produce an export warning because their live descriptors
  are unsupported. The PSD retains the rendered cached appearance, but the unsupported
  live filter settings are omitted. Keep the `.pcraft`
  original for editable work. Generated PSDs are tested with PhotoCraft's strict
  parser and importer; third-party acceptance is not established by those tests.
- The six Paint.NET-specific filter blend modes have no PSD equivalent. Export warns
  that their live descriptor mode is written as Normal; the native document and PSD
  cached appearance remain unchanged. Keep the `.pcraft` original to preserve the mode.
- Enabled stacks containing visible unsupported filters in imported files block adjustment
  creation and parameter editing. Other existing stack actions retain their behavior. Hiding that
  filter explicitly permits a new rendering; the unknown descriptor remains stored.
- Missing linked sources must be relinked before editing.
- Painting currently refuses nonzero filter-mask feather. Feather-aware painting
  and mask-property controls are the next workflow; density/feather PSD interchange
  is still incomplete. Other painting tools explicitly refuse this mask target.
- Drag reordering stays within one layer. Other filter editors retain their existing
  preview limitations; these stack controls do not establish all blend-mode or
  proprietary filter-mathematics parity.
- The blending dialog warns that Smart Filter Dissolve currently uses continuous
  blending; it does not yet implement
  the deterministic coverage used by the layer compositor's Dissolve mode. Non-Normal
  CMYK/Lab filter blending also lacks an independent proprietary reference.
- Reduced previews are supported for stacks containing these four adjustments.
  Other filter kinds, layer effects or enabled vector masks retain full-resolution previews. Reduced previews
  approximate sampling and neighborhood effects; committed processing is full size.
- Curves eyedroppers use the existing committed merged-image sampler. They do not
  sample the isolated pixels below the selected Smart Filter.
- Colorize no longer imposes a hidden 25% minimum saturation in either the CPU or
  GPU compositor. In Normal blend mode, Smart Filter opacity interpolates image
  coverage instead of stacking a second translucent copy. Other modes retain their
  source-over coverage, checked against the pinned Screen Smart Filter oracle.
  These fixes do not establish general proprietary
  rendering equivalence or raise corpus fidelity floors.

The synthetic acceptance suite lives in `crates/engine/tests/smart_adjustments.rs`;
dialog and scaled-placement regressions are in
`crates/ui-egui/src/smart_adjust_preview.rs`. Mask painting and stale-cache coverage
are in `crates/engine/tests/smart_filter_masks.rs` and
`smart_filter_mask_rebase.rs`; mask and stack interaction checks are in the UI's
`smart_mask_tests.rs` and `smart_filter_rows_tests.rs`.
