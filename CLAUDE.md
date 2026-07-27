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
- Oracle = headless LibreOffice PNG render. It is imperfect but consistent; treat large diffs as regressions, small ones as tolerance.
- Add a fixture for every new feature BEFORE implementing it. Golden tests must stay green.
- Per-suite tolerances live in `tests/golden/suites.json`, each with a written reason. Widening one is a decision to record, not a knob to turn.
- `cargo test` covers layout logic against a synthetic measurer (`StubMeasure`), so it asserts *where the breaks fall* without depending on which fonts a machine has. Pixel fidelity is the golden suite's job.
- When a golden diff appears, get `debugTrace()` for the slide first. It tells you in one look whether layout moved something or the rasteriser drew it differently — the most common ambiguity in this project.

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
| **the whole viewer as it stands** | **592 KB** | **265 KB** |
| baseline + `cosmic-text` 0.14, shaping one line | 788 KB | 302 KB |
| ⇒ cosmic-text's own contribution | ~761 KB | **~293 KB** |

Adding `cosmic-text` would more than double the download — and that is *before* fonts. The
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
