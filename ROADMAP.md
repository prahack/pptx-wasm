# Roadmap

Where this renderer stands against the field, what would actually move it, and in what
order. Every claim here is a measurement from `npm run compare` or `npm run bench`, not an
estimate — where something is an estimate it says so.

## Where we actually stand

Of seven browser pptx renderers, on the 11 fixtures all of them render:

| axis | rank | note |
|---|---|---|
| warm render | **1 of 7** | 2.0 ms; next is 3.0 ms (@jvmr, at 46.58% structural error) |
| cold start | 2 of 7 | 31.0 ms; only @jvmr is faster, and it renders far less |
| fidelity (structure) | **1 of 7** | 1.15% vs @aiden0z's 1.37%; the rest of the field is 12–59% |
| fidelity (text) | 5 of 7 | 27.43%, the weakest of the five accurate engines, inside a 3-point band |
| payload | 4 of 7 | 319 KB; @jvmr 45 KB, glimpse 167 KB, pptxviewjs 252 KB |

Restricted to the five engines that render accurately at all, we are **first on cold and
first on warm**. So the honest summary is: most accurate on structure, fastest of the
accurate engines, mid-pack on size, and marginally last on text among that same group.

Fidelity is reported split because pooling it destroys the signal: text-dominated fixtures
put every competent engine within three points of the rest, so averaging them in drags a
1.15% structural result up to 20% and makes a 50x spread look like a rounding error.

That shapes the roadmap. We do not need to get faster. We need to stop losing on size, and
we need to close the one capability gap that no amount of speed compensates for.

---

## P0 — the gaps that cost us adoption

### 1. Text selection and accessibility

**The single biggest competitive gap, and it is architectural.** A canvas renders pixels:
text cannot be selected, copied, found with Ctrl-F, or read by a screen reader. Every
DOM-based competitor gets all four for free. For a viewer — where reading is the entire
use case — that is a serious objection, and no benchmark row captures it.

The fix does not require abandoning canvas. The display list already carries per-glyph
positions and per-character advances (they exist so the WebGPU backend can position glyphs
without a shaper). That is everything needed to lay transparent, correctly-positioned
`<span>`s over the canvas — the same technique pdf.js uses for its text layer.

- Emit a text-layer description from the display list: string, rect, font size, transform.
- Render it as absolutely-positioned transparent text above the canvas.
- Gate it behind an option; it costs DOM nodes on dense slides.
- Verify with an actual screen reader, not by assuming.

Impact: removes the one reason to pick a DOM renderer over this. Effort: medium. Risk:
low — purely additive, cannot regress the raster path.

### 2. The flowchart and action-button presets

Found by reading @aiden0z/pptx-renderer — the one engine whose fidelity is close to ours —
to see what it does differently. It supports **43** `flowChart*` and `actionButton*`
shapes. We support **9**. The other **34** fall back to their bounding rectangle.

A probe deck of twelve ordinary flowchart shapes renders like this:

| this renderer | @aiden0z/pptx-renderer |
|---|---|
| ![ours](docs/flowchart-ours.png) | ![theirs](docs/flowchart-aiden0z.png) |

Ten of the twelve are plain blue rectangles. Only `flowChartDecision` and `flowChartData`
come out right, because a diamond and a parallelogram happen to exist for other presets.

This matters more than the count suggests. It is the exact failure mode this project
criticises @jvmr/pptx-to-html for in the comparison — "hexagon, diamond, chevron, star,
plus, can, cube and donut all fall back to a plain rectangle" — and process diagrams are
one of the most common things in a business deck. It also does not show up anywhere in the
benchmark, because no fixture contains a flowchart. **The structural score of 1.15% is
measured on shapes we chose to implement.**

Two things to do, in order:

1. Add a flowchart fixture to `fixtures/gen.py` and to the golden suite, so the gap is
   visible in the numbers rather than only in a probe.
2. Implement the 34 missing presets from the ECMA-376 formulas. They are mostly simple
   polygons and arcs — far easier than the star and hexagon work already done — and
   `is_supported()` already exists to assert coverage rather than let it regress silently.

### 2b. Refine five preset curves — *follow-up from 2*

The 22 new presets landed and `m7a` now diffs 2.16% against the oracle, but that residue
is not evenly spread. Measured per shape: `multidocument` 18.4%, `document` 14.4%,
`punchedTape` 11.1%, `terminator` 10.8%, `inputOutput` 9.8%. The other 24 are all under
5.2% and mostly pure antialiasing.

The interesting part is that **three of those five — `document`, `terminator` and
`inputOutput` — predate the flowchart work entirely.** They shipped in the first
milestone, were never covered by a fixture, and nobody knew their wave, corner radius and
slant were wrong. Adding coverage for a gap found a second gap underneath it.

Each is a small correction against the reference render, and `m7a`'s tolerance should come
down to under 1% once they land.

### 3. Payload: 319 KB → target ~220 KB

Fourth of seven is the only axis where we lose to engines that also work. Two facts
already measured, so the obvious moves are ruled out:

- The release profile is already `opt-level="s"`, `lto=true`, `codegen-units=1`,
  `panic="abort"`, `strip=true`.
- `wasm-opt -Oz` instead of `-O3` saves **1 KB gzipped**. Not worth the speed risk.
- Dependencies are already trimmed: `quick-xml` with no default features, `zip` with
  `deflate` only.
- 89.7% of the binary is the code section. It is our own compiled Rust, not metadata.

So the remaining lever is **compiling less of it**, via Cargo features:

- `charts` — `layout/chart.rs` and `parse/chart.rs` are 2,355 lines, ~11.5% of the core.
  Most decks in a viewer context have no charts.
- `effects` — shadows, glow, soft edges.
- `tables` — smaller, but self-contained.

Ship `pptx-wasm` with everything on (unchanged for existing users) and let people opt
down. **Measure with `twiggy` before cutting anything** — the 11.5%-of-source figure is a
proxy for binary size, not a measurement of it, and monomorphised generic code does not
track line count.

### 4. CI

There is none. The last two sessions found five real bugs — four preset-geometry errors
and a tiled-fill regression — that the golden suite either hid behind a loose tolerance or
would have passed through entirely. That suite is the main defence against exactly this
class of error, and nothing runs it automatically.

- GitHub Actions: `cargo test`, `clippy`, `fmt`, then the golden suite.
- LibreOffice and poppler in the runner image; cache the `.venv` and the Cargo registry.
- Run `npm run compare` on a schedule, not per-PR — it takes minutes and depends on
  third-party packages that change under us.

Effort: low. Value: high — this is what keeps the fidelity number honest between releases.

---

## P1 — fidelity confidence

### 5. Validate against PowerPoint itself

The known gap recorded in `CLAUDE.md` and never closed. Everything has been validated
against LibreOffice, which leaves a whole class of error uncaught: cases where LibreOffice
and this renderer are wrong in the *same* way, or where the implementation has been tuned
toward LibreOffice's quirks.

Cheap to do and worth doing before trusting any tolerance: export `m2-text.pptx` and
`m4-template.pptx` from real PowerPoint, keep the PNGs, spot-check by eye against
`tests/golden/out/actual/`.

There is a concrete open question waiting on this. On `m4`, our text renders ~8% larger in
ink extent than LibreOffice's, while our resolved sizes (44/32/28 pt) match the slide
master exactly. That is consistent with font substitution rather than a resolution bug —
but it has not been confirmed against PowerPoint, and until it is, "our sizes are right"
is an argument rather than a fact.

### 6. EMF/WMF images

No browser decodes these, and they are common in real decks — anything pasted from Excel
or older Office arrives as a metafile. Today they render as their fallback image where the
file provides one, and as nothing where it does not.

This is genuinely hard, and it is worth noting that the ChristopherVR project ships a
dedicated `emf-converter` package rather than solving it inline. Options, cheapest first:
prefer the fallback raster more aggressively; parse the EMF subset that covers pasted
charts; or treat it as out of scope and document it clearly.

### 7. The remaining coverage gaps

In rough order of how often they appear in real decks: soft edges, 3-D bevels, SmartArt
(currently falls back to its cached image), OLE embeddings, animations and transitions.
Animations may be permanently out of scope for a *read-only viewer* — worth deciding
explicitly rather than leaving on a list forever.

---

## P2 — reach

### 8. An SVG backend

Strategically the highest-leverage item on this list, because one backend closes three
gaps at once:

- **Text selection and accessibility**, natively, with no overlay hack.
- **Server-side rendering** — thumbnails from Node, with no browser and no canvas polyfill.
- **Resolution independence** — print and export at any size.

The `Renderer` trait already exists precisely to make this a contained change; `canvas2d`,
`record` and `webgpu` all implement it, and the display list is deliberately
backend-agnostic and in points. An SVG backend has to satisfy the same golden tests, which
is the guard that keeps it honest.

This overlaps with P0.1. If the SVG backend happens, the text-layer overlay may be
unnecessary — worth deciding which before building either.

### 9. Framework wrappers and a demo

- Vue and Svelte wrappers. The core is framework-agnostic; only React has a wrapper today.
- A hosted demo. `examples/comparison` already renders seven engines side by side on a
  user-supplied deck, which is a genuinely persuasive artefact and currently only runs
  locally.
- Docs beyond the two READMEs.

---

## Explicitly not doing

- **Getting faster.** We are already first among accurate engines on both timings. Effort
  spent here buys nothing a user notices.
- **A Web Worker.** Measured and rejected in `CLAUDE.md`; the numbers that would justify
  revisiting are written down there.
- **Editing.** Read-only is the product. Several competitors are editors, which is most of
  why they are 3-25× larger.

---

## Suggested order

0. ~~Flowchart presets~~ — done: 22 implemented, `m7a` fixture added, 19/19 golden green.
1. CI — cheap, and everything after it is safer with it in place.
2. `twiggy` measurement, then chart/effects feature flags.
3. PowerPoint spot-check — resolves the open `m4` question.
4. Decide SVG backend vs. text-layer overlay, then build the winner.
5. Wrappers and the hosted demo.

---

## What reading the competition actually taught us

Worth recording, because the useful findings were not the ones the benchmark surfaced.

**@aiden0z/pptx-renderer is a hybrid: SVG for shapes, HTML for text, no canvas at all**
(95 `createElementNS` calls, zero `getContext('2d')`). That single choice buys it text
selection, Ctrl-F and screen-reader support for free — the P0.1 gap here — and costs it
3x on warm render, 6.0 ms against 2.0 ms. It is the clearest evidence that the SVG backend
in P2.8 is the right shape of idea, and that it will not be free.

**Its text is measured with `measureText` exactly as ours is**, then emitted as DOM and
left to CSS for final line breaking, where we compute wrap points ourselves and place each
run. It scores 26.93% on text against our 27.43%. Half a point inside a three-point noise
band is not evidence of anything, and no conclusion should be drawn from it — but it is
the only axis where the architectures visibly diverge, so it is where to look if the text
figure ever becomes a real gap.

**The benchmark did not find the flowchart hole; reading their source did.** A comparative
score can only measure the fixtures you thought to write, and every fixture here was
written by the same person who chose which presets to implement. That is a blind spot the
numbers cannot see past, and it is worth periodically diffing capability lists against a
competitor rather than only diffing scores.
