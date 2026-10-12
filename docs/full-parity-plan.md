# PhotoCraft parity completion plan

This is the execution inventory for the continuation begun on 2026-10-12, after
[the editable Smart Object adjustment milestone](smart-adjustments.md). It is a
backlog and evidence plan, not a claim of Photoshop equivalence.

## What the current measurements mean

The eight `scorecard/*.toml` checklists currently declare **159 rows: 62 done,
43 partial and 54 missing**. These are the starting snapshot. Several descriptions
predate implementations now in the source, so their labels need reconciliation
against code, regression tests and complete user workflows. Examples include Smart
Object perspective, Stamp Visible, manual/Optical kerning, copying whole layers
between documents and Actions-panel editing. Existing code alone does not establish
that the full criterion works.

The generated [scorecard](scorecard.md) also reports 47 of 146 preferences without
readers. Its committed performance baseline is from 2026-10-05 on a heavily loaded
Apple M4 Pro: three of 56 scenarios meet their targets, 22 exceed a budget or fail,
15 were not run, seven are not measurable yet and nine are tracked without a
budget. These numbers do not describe performance on the current Ubuntu machine.
The current menu catalogue has 627 wired entries; menu wiring is not behavioral
coverage.

The previous milestone passed installed workflow checks on Ubuntu 24.04 and 26.04,
under X11 and Wayland with software rendering. This establishes those tested
workflows and configurations, not physical NVIDIA/Intel GPU or Windows validation.

## Remaining dimensions

| Dimension | Concrete remaining work | Evidence needed |
|---|---|---|
| Smart Objects and filters | Hue/Saturation and Colorize creation/re-edit; paintable filter masks; useful mask thumbnails/selection; stack collapse, blending, reordering and context controls; missing live PSD descriptors; other filter-editor previews | Source pixels unchanged; complete parameters, cancel and one undo step; mask/channel/lock routing; native roundtrips; explicit PSD warnings; rendered and interaction tests |
| PSD rendering and interchange | Unmatched filter mathematics, placement sampling, effects/strokes, advanced blending, rounding, color models, text and opaque imported features | Existing pinned corpus floors must not regress; inspect differences against licensed reference images; independent third-party open/edit/export checks where available |
| Other file and output workflows | Layered TIFF extra channels/Quick Mask; vector and multipage PDF workflows; spot inks/plates; RAW formats, sidecars, calibration and per-camera fixtures; video interchange; export settings; local asset libraries; artboard depth | Synthetic roundtrips and standards-based fixtures, independent parsers/renderers, realistic document workflows, documented loss and unsupported cases |
| Painting and retouching | Live previews for remaining retouch tools; Clone Source gaps; pressure curves and image brush tips; Content-Aware Move result preview and Transform On Drop | Several depths, selection boundaries, alpha edges, cross-tile strokes, source/undo preservation, pen hardware tests and latency/memory measurements |
| Selection, crop, paths and transforms | Dedicated Select and Mask / Content-Aware Fill / Vanishing Point workspaces; Move and Free Transform interactions; remaining pen variants/modifiers, crop units/presets, path-stroking choices, shape handles and mask controls | Complete pointer/keyboard/control workflows, cancellation and history, geometry/selection oracles and visible previews |
| Type and typography | Text on a path; complete direction/RTL caret behavior and complex-script fallback; variable/OpenType UI; verified manual/Optical kerning workflows; Warp Text current-state/live-preview behavior; text transform/rasterization choices | Shaping and caret goldens from redistributable fonts, style-preserving edits, native/PSD roundtrips, licensed reference output and realistic typography workflows |
| Workspace and UI | Inert preferences; persistent settings and recents; floating/restored panels; scale/themes/localization/RTL; numeric-field units and arithmetic; preset management; remaining Layers/Color/Curves interactions | Preference reader and restart tests, keyboard/pointer automation, visual checks at several scales/languages, no misleading controls |
| Automation and ecosystem | Document-independent actions and interchange; replay/batch depth and cancellation; expanded plug-in capabilities; 30–50 substantial acceptance tasks; scripting compatibility | Equivalent direct/UI/CLI/MCP outcomes, second-document replay, authority boundaries and cancellation; clearly separate native plug-ins from Adobe-specific APIs |
| Reliability and large documents | Allocation-failure handling, large-file/layer-count fixtures, atomic-save failure cases and sustained stress checks | Failure injection preserves document/history and disk originals; bounded hostile-input tests; explicit memory limits and recovery behavior |
| Performance | Complex-layer interactions, GPU texture-limit behavior, expensive filter/selection jobs, cancellation and responsiveness; missing measurement harnesses | Repeatable 24–36 MP release benchmarks and frame/memory measurements on named hardware; distinguish regression comparisons from budget attainment |
| Platforms and distribution | Native Wayland tablet input, real Windows/Linux GPU and tablet testing, signing/update/report flows, ARM and other supported packaging targets | Installed acceptance on each supported OS/backend, physical-device checks and reproducible provenance; software rendering cannot certify hardware behavior |
| AI and proprietary services | Local model selection/management and generative features; Adobe-specific fonts/libraries/cloud documents and scripting services | Product/backend decisions, licensed models/assets and service access. These cannot be inferred from menu wiring or supplied by a self-roundtrip test |

The detailed row backlog remains in [the scorecard sources](../scorecard/) and
[roadmap](roadmap.md). A stale row is corrected only with named implementation and
test evidence; a broad row remains partial while meaningful clauses are open.

## Active workflow queue

1. **Complete native Hue/Saturation and Colorize Smart Filters.** Preserve all six
   ranges and custom bounds; verify re-editing, low/zero Colorize saturation, nested
   parameter patches, replacement previews, masks, color models, history and native
   persistence. The `hue2` PSD adjustment-layer format is not proof of a Hue/Saturation
   Smart Filter descriptor. Until that descriptor is supported, PSD export must warn
   and retain the cached rendered appearance without pretending to retain live editing.
2. **Paint Smart Filter masks directly.** Reuse Brush/Pencil/Eraser behavior, bind the
   document/layer/authority for a gesture, update only damaged preview regions, preserve
   the source/filter stack, and commit mask plus rendered cache in one undo step.
   Nonzero filter-mask feather is a separate rendering gap: existing Smart rendering
   ignores it, so the first painting workflow must refuse it explicitly rather than
   silently changing its meaning. Native density and default FEid PSD mask behavior
   need explicit regression tests; mask density/feather PSD interchange is not complete.
3. **Complete Smart Filter stack controls.** Collapse, meaningful thumbnails and
   targeting, blend controls, drag reordering and context actions must use the existing
   command/history/authorization boundaries and remain fully controllable by automation.
4. **Finish filter-mask feather and density.** Follow the painting/stack checkpoint
   with native mask properties and feather-aware rendering and local painting damage.
   Reuse the existing public-specification Gaussian behavior and verify halos, image
   edges, defaults and HDR samples. Preserve PSD appearance with an explicit baking
   warning wherever a verified live descriptor is unavailable; do not invent fields.
5. **Continue through the remaining workflow gaps.** Prioritize concrete blockers in
   Select and Mask, content-aware editing and Warp Text before surface-only additions.
   Reconcile existing kerning/actions/transform implementations with their stale rows,
   add missing acceptance coverage, and then work through output, preference and
   performance gaps above. Each completed workflow gets a bounded reviewed commit;
   a completed native workflow does not close a separate interchange or hardware gap.

## Verification boundary

Public specifications, synthetic fixtures and redistributable reference corpora are
valid implementation inputs. Tests against PhotoCraft's own destructive engine or
export/import paths establish internal consistency; they do not establish independent
Photoshop equivalence. This environment does not provide an installed Photoshop
reference, proprietary service access or the user's physical GPU/tablet hardware.
Those verification gaps stay visible while implementable work continues. No overall
parity percentage is raised from the number of commits, menu entries or passing tests.
