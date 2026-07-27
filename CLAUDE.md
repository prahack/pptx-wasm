# PPTX Viewer

Read-only browser PPTX renderer. Rust → WASM core, TS API, React wrapper. No backend.

## Architecture (do not violate)
- Pipeline: ZIP reader → OOXML parser → Presentation Model → Layout Engine → Display List → Renderer → Canvas.
- The Display List is resolution-independent and backend-agnostic (positioned glyphs, filled/stroked paths, images-with-transform). Layout NEVER talks to a canvas directly.
- Renderer is a trait. Backends: `canvas2d` (default, browser 2D context) and `webgpu` (target). New backends must satisfy the same golden tests.
- Coordinates: OOXML uses EMUs (914400 EMU = 1 inch). Convert to px at the viewport boundary only. Keep the model in EMUs.
- Inheritance resolution order for any property: shape → placeholder → slide layout → slide master → theme default.

### Two conventions that are load-bearing

**Display-list coordinates are points, not pixels.** Layout converts EMU → pt once and stops.
The renderer's `View` turns points into device pixels. That is what lets zoom, resize and
DPR changes re-render *without re-laying-out* — and it is why wrap points cannot drift
between zoom levels. If you find yourself putting a pixel value in a display list, the
design has gone wrong.

**`None` means "not specified", not "default".** Every inheritable property in the model is
an `Option` (or a `Fill::Inherit`). The whole resolution chain depends on telling an unset
property from one explicitly set to a default value: `<a:rPr b="0"/>` on a slide must beat
`b="1"` on the master, and it can only do that if `Some(false)` and `None` are different.

## Layout
- crates/core        Rust: parser, model, layout, display list
- crates/renderer    Rust: Renderer trait + canvas2d + webgpu backends
- crates/wasm        wasm-bindgen entry, TS-facing API
- packages/viewer    TypeScript API layer + React `<PresentationViewer/>`
- fixtures/          fixture generator (python-pptx) + generated .pptx
- tests/golden/      oracle renderer, snapshot runner, pixel-diff

## Commands
- Build WASM:      `npm run wasm`  (`wasm-pack build crates/wasm --target web`)
- Dev server:      `npm run dev`
- Rust tests:      `cargo test --workspace`
- Golden tests:    `npm run test:golden`   (regenerates fixtures, renders, diffs)
- One suite:       `npm run test:golden -- --suite=m2`
- Regen goldens:   `npm run goldens:update` (only after visual review)
- Cross-browser:   `npm run test:browsers` (Chromium, Firefox, WebKit)
- Bench:           `npm run bench`  (`-- --fixture=bench-dense.pptx` for the dense case)

The browser-driven commands start a dev server themselves if one is not already running.

Toolchain the golden suite needs: `.venv` with `python-pptx` (fixture generation),
LibreOffice (the oracle), `pdftoppm` from poppler (page rasterisation), Playwright
Chromium (rendering the viewer). `npm run test:golden` reports which are missing and
degrades rather than failing outright — see "Testing philosophy".

## Conventions
- Parsing uses `quick-xml`; container uses `zip`. No serde-xml (too slow/loose for OOXML).
- Every parse function returns a value and degrades gracefully; a malformed slide renders as much as it can, never panics.
- No `unsafe` (the core denies it outright). No panics in the core: `unwrap`/`expect`/`panic` are denied outside tests.
- Element matching is by *local* name; attribute matching is prefix-exact, with `r_attr` for the `r:` namespace. `<p:sldId id="256" r:id="rId2"/>` has two attributes whose local name is `id`.
- Commit per task with a message referencing the milestone (e.g. "M2: paragraph wrapping").

## Testing philosophy
- Oracle = headless LibreOffice PNG render. It is imperfect but consistent; treat large diffs as regressions, small ones as tolerance. See "why LibreOffice" below for what it is and is not authoritative about.
- Add a fixture for every new feature BEFORE implementing it. Golden tests must stay green.
- Per-suite tolerances live in `tests/golden/suites.json`, each with a written reason. Widening one is a decision to record, not a knob to turn — and *tightening* one after a fix is part of the fix, not a separate chore. A tolerance far above the actual figure is not a safety margin, it is a blindfold: m3 sat at 3.5% while hiding four visibly wrong shapes, none of which moved it past 1.3%.
- **Preset geometry comes from ECMA-376's formulas, not from eyeballing the shape.** Every preset bug found so far was a plausible-looking construction that no diff caught: a star inscribed in the box's ellipse rather than fitted to it, `adj` read over 100000 where the spec says 50000, a head or inset scaled by the width where the spec says `ss` (the shorter side). When adding or fixing one, look it up. Prefer deriving a factor to copying it: `fill_box` measures the extent instead of hard-coding `hf`/`vf`, and reproduces the spec's constants exactly.
- `cargo test` covers layout logic against a synthetic measurer (`StubMeasure`), so it asserts *where the breaks fall* without depending on which fonts a machine has. Pixel fidelity is the golden suite's job.
- When a golden diff appears, get `debugTrace()` for the slide first. It tells you in one look whether layout moved something or the rasteriser drew it differently — the most common ambiguity in this project.

---

## Decision: LibreOffice as the oracle  *(M0, settled — with a known gap)*

**Decision: headless LibreOffice is the golden-test reference. It is authoritative where
the spec is determinate and a second opinion where it is not.**

### Why an oracle at all

Without an independent reference, "does this render correctly?" resolves to the author's
own judgement, which is the thing that drifts over a build this size. Unit tests can assert
that a line breaks between word four and word five; they cannot tell you the whole deck
looks wrong because tint is being computed in the wrong colour space.

### Why LibreOffice

Three constraints, and it is the only thing that meets all three:

- **Headless and scriptable.** PowerPoint needs Windows, a licence and COM automation. It
  cannot run in this loop at all.
- **An independent implementation of the same spec.** This is the one that matters.
  LibreOffice was written by people reading ECMA-376, not by us, so a disagreement means
  one of us has misread it — and that is information. A self-snapshot baseline can only
  tell you something *changed*, never that it was wrong from the first commit.
- **Deterministic and version-pinnable.**

The route is `.pptx → PDF → per-page PNG` via `pdftoppm`, because `--convert-to png` only
renders the first slide.

### What it has earned

The linear-light tint bug. `m5a` was 36% off; rather than widen the tolerance, the oracle's
banded-table pixels were sampled and the formula derived from them. They match `accent1`
tinted in **linear** light on all six channels to within one count; the sRGB-byte lerp that
was implemented is off by 20-30 per channel. That bug affected every banded table and every
"Lighter 40%" theme colour in every deck, and reading the spec had already produced the
wrong answer once — with a confident comment explaining why. It took an independent
implementation disagreeing.

### Where it is *not* authoritative

`m5b` carries a 15% tolerance against 0.1% elsewhere. `chart.xml` holds data and formatting
but no geometry, so LibreOffice picks its own axis scale, gap widths and label placement,
none of which the file specifies. It chooses 200-unit ticks to 1600; we choose 500 to 1500.
Neither is wrong.

Hence the rule: authoritative for colours, positions, geometry and inheritance; a second
opinion for auto-scaled chart axes, Gaussian blur radii and font hinting. That is why every
tolerance in `suites.json` carries a written reason rather than only a number.

### Where it is *wrong*, which is a different problem

`m5b` is a case of the oracle making a defensible different choice. `m5f` is not:
**LibreOffice ignores `<a:tile>` on a blip fill and stretches the image instead** — the
exact bug the fixture exists to catch.

That inverts the test. A correct repeating render diffs ~4% against the oracle; a
regression back to stretching would diff near *zero*. The suite would have been loudest
when the feature was working and silent when it broke — worse than having no test, because
it manufactures confidence. It passed at 4.4% under a 6% tolerance, and would have gone on
passing through the regression it was written to catch.

So a suite can set `"oracle": false` and compare against a reviewed reference in
`tests/golden/out/reference/` instead. That detects regressions without pretending to
verify fidelity, and the runner labels every such line `[vs recorded reference, not the
oracle]` so the weaker guarantee is visible at the point the number is read. Fidelity for
those features is argued from ECMA-376 and asserted in unit tests.

The general lesson, and the reason this is written down: **before trusting a tolerance,
check that the oracle renders the feature at all.** A passing diff against a reference that
does not implement the thing being tested says nothing. When adding a suite for a new
feature, look at the oracle's PNG once, by eye, before accepting the number.

### The known gap

**Nothing here has been validated against PowerPoint itself.** That leaves a class of error
uncaught: cases where LibreOffice and this renderer are wrong in the same way, or where the
implementation has been tuned toward LibreOffice's quirks.

The fix is cheap and worth doing on the milestones where fidelity matters most: export
`m2-text.pptx` and `m4-template.pptx` from real PowerPoint, keep the PNGs, and spot-check
them against `tests/golden/out/actual/` by eye. Do this before trusting a tolerance that
had to be widened.

---

## Decision: Spike A — text backend  *(M0, settled)*

**Decision: Canvas2D `measureText` is the metrics source. `cosmic-text` is rejected for
now, and no code depends on which was chosen.**

Layout asks a `TextMeasure` (`crates/core/src/text/`) for advance widths. The wasm crate
supplies `CanvasTextMeasure`, which measures through an `OffscreenCanvas` 2D context.

### Why

*Correctness.* The Canvas2D renderer draws with `fillText`. Measuring through the same
engine means the advances layout wrapped against and the glyphs the browser puts on screen
come from one source. Any other measurer introduces a class of bug where the line breaks
in a place the drawn text does not actually reach.

*Payload.* Measured on this machine, release profile, `wasm-opt -O3`:

| build | raw | gzipped |
|---|---|---|
| empty `wasm-bindgen` crate (baseline) | 27 KB | 9 KB |
| **the whole viewer, M0–M5 complete** | **710 KB** | **313 KB** |
| baseline + `cosmic-text` 0.14, shaping one line | 788 KB | 302 KB |
| ⇒ cosmic-text's own contribution | ~761 KB | **~293 KB** |

Adding `cosmic-text` would roughly double the download — and that is *before* fonts. The
browser will not hand us its font files, so deterministic metrics also mean shipping the
faces to measure against: a single Carlito WOFF2 is ~120 KB, and a set covering the
Calibri/Cambria/Arial/Times substitutions in `text::fallbacks_for` is comfortably 500 KB
more. Roughly 800 KB gzipped to make wrap points identical across browsers.

### What we give up, and what we do instead

Cross-browser metric stability. Chrome, Firefox and Safari do not return byte-identical
`measureText` widths for the same face, so a paragraph can in principle break one word
differently between them.

Two things blunt this. Advances are measured in **points at the authored size**, never at
the zoomed size, so the same deck breaks identically at every zoom level and DPR in a given
browser — the common case for "it moved". And `text::fallbacks_for` puts *metric-compatible*
substitutes ahead of generic families (Carlito for Calibri, Liberation Sans for Arial), so
a missing face keeps its wrap points rather than reflowing the slide.

### What the evidence says so far

`npm run test:browsers` renders every fixture in Chromium, Firefox and WebKit and compares
them. On the current suite — including the deliberately wrap-heavy `m2` — the extracted
text is **identical in all three**, and the proportion of non-background pixels agrees to
within 0.2%. So the theoretical divergence has not produced a single differing wrap point
yet. That is the check to run before reopening this decision.

### When to revisit

If cross-browser wrap differences show up as real bugs rather than a theoretical concern.
The seam is the `TextMeasure` trait; a `cosmic-text` implementation slots in behind it with
no change to layout. The `text-cosmic` feature on `crates/wasm` is the placeholder, and it
`compile_error!`s today rather than silently shipping a *worse* measurer than the default —
cosmic-text without embedded fonts is not more deterministic, just differently wrong.

---

## Decision: Spike B — renderer abstraction  *(M0, settled)*

**Decision: `Renderer` is a command-at-a-time trait; `canvas2d` ships, `webgpu` is a
measured stub.**

`crates/renderer/src/lib.rs` defines the trait and the single `render()` walk over a
display list. Three implementations exist:

- **`canvas2d`** — the shipping backend. Browser-only (`cfg(target_arch = "wasm32")`).
- **`record`** — serialises commands to text. Runs on the host, which is what lets
  `cargo test` assert on renderer behaviour with no browser, and what `debugTrace()`
  exposes for diagnosing golden diffs.
- **`webgpu`** — deliberately *not* a drawing backend yet. It implements the same trait but
  measures instead: `Requirements::analyse()` reports what a GPU backend would need for a
  given slide (paths to tessellate, glyphs to atlas, scissor vs. stencil clips) and flags
  anything the display list cannot express. Its test suite is the guard that keeps the
  abstraction honest — if an upstream change makes a display list un-renderable on the GPU,
  it fails there and then, rather than being discovered whenever Backend B is attempted.

The display list carries per-character advances alongside the string precisely so both
backends can be served: Canvas2D hands the string to the browser's shaper, WebGPU positions
glyphs from the advances. Whoever produced the advances is also the authority on line
breaking, so the two backends break lines identically even though they rasterise
differently. **A text run without advances is a bug** — `webgpu::Requirements` reports it as
unsupported for exactly this reason.

WebGPU text is optional to shipping. If it stalls, Canvas2D ships; the abstraction exists
so that stays a cheap decision.

---

## Decision: no Web Worker  *(M6, settled — revisit if the numbers change)*

**Decision: parsing and layout stay on the main thread. Culling was the win instead.**

M6's task list included moving parsing and layout into a Web Worker. Measured on this
machine (`npm run bench`, release wasm, headless Chromium), it is not warranted:

| | 250-slide deck | 2000-shape slide |
|---|---|---|
| open + parse index | 16 ms | 12 ms |
| first slide (layout + draw) | 17 ms | 81 ms |
| navigate to an unvisited slide | 0.4 ms | — |
| zoom (cached display list) | 0.1 ms | 14 ms |

Slide count barely matters, because parsing is lazy: opening a deck reads the package
index and `presentation.xml`, nothing else. The only figure near a frame budget is drawing
a genuinely dense slide.

**A worker would not have helped that figure.** The 14 ms is Canvas2D draw calls, and those
have to happen on whichever thread owns the canvas — moving the work behind an
`OffscreenCanvas` relocates the same milliseconds. What did help was
`renderer::cull`: skipping commands that cannot affect a visible pixel took the dense
slide's zoom from 17.6 ms to 13.8 ms, and the golden suite confirmed it changed no pixels
(every suite's diff ratio was identical before and after).

### When to revisit

If any of these becomes true, the worker is back on the table:

- Opening a deck exceeds ~100 ms — likely with very large embedded media, since the OPC
  index is proportional to part count.
- A single slide's *layout* (not its drawing) exceeds a frame. Layout is the part that
  genuinely parallelises.
- Text measurement stops being cacheable — a deck with thousands of distinct
  (string, font) pairs would make `CanvasTextMeasure` chatty across the wasm boundary.

`Presentation.measureCalls()` exists to catch the last one: if it keeps climbing across
re-renders of the same slide, the metrics cache is being defeated.
