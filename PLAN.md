# PPTX Viewer — Claude Code Execution Plan

This is a driver's plan for building the browser-native PPTX viewer with Claude Code.
It reshapes the original vision doc into agent-executable milestones. Feed the **Kickoff
Prompt** first, commit the **CLAUDE.md** into the repo, then work milestone by milestone.

---

## Key changes from the original vision doc (read this first)

1. **Vertical slices, not horizontal layers.** The original phases finished all parsing,
   then all text, then all compatibility. That hides integration bugs until the end. Here,
   M1 renders a real (trivial) PPTX end-to-end — zip → parse → model → layout → pixels →
   navigation — and every later milestone deepens a working pipeline.

2. **Renderer is an abstraction, not "WebGPU."** A custom GPU text renderer (shaping +
   font metrics + rasterization) is the biggest risk in this project. The plan introduces
   a `Renderer` trait fed by a resolution-independent **display list**. Backend A
   (Canvas2D bridge) gets you to accuracy fast and uses browser fonts; Backend B (WebGPU
   glyph atlas + tessellation) is the ambitious target you swap in behind the same
   interface. You never block shipping on the hard backend.

3. **The agent must be able to check its own work.** Every milestone has a golden-file
   test: generate fixtures with `python-pptx`, render a reference PNG with headless
   LibreOffice (`soffice --convert-to png`), and pixel-diff against the viewer output.
   Without this loop Claude Code has no signal and will drift.

4. **One central architecture tension, decided in M0:** deterministic cross-browser text
   (your success criterion #7) argues for in-Rust layout via `cosmic-text`; ease and speed
   argue for the browser's own text engine via Canvas2D `measureText`. M0 spikes both and
   records the decision. Don't let the agent pick this implicitly.

---

## Kickoff Prompt (paste into Claude Code first, in Plan Mode)

> We're building a browser-native, read-only `.pptx` viewer: Rust core compiled to WASM,
> a TypeScript API layer, and a React wrapper component. No server-side conversion.
>
> Read `CLAUDE.md` and `PLAN.md` (this file) before writing code. Work milestone by
> milestone in order. For each milestone: (1) restate the definition of done, (2) propose
> the task breakdown and let me approve, (3) implement task by task with a commit per task,
> (4) run the golden-file test suite and show me the pixel-diff results before moving on.
>
> Non-negotiable invariants: the layout engine emits a backend-agnostic display list; all
> rendering goes through the `Renderer` trait; no `unsafe` without a comment justifying it;
> no network calls in the core; every parser handles missing/malformed XML without panicking.
>
> Start with Milestone 0. Do not skip the decision spikes.

---

## Milestone 0 — Bootstrap + decision spikes

**Goal:** a running (blank) viewer, the test harness, and the two hard decisions made.

Tasks
- Scaffold the Cargo workspace (`core`, `renderer`, `wasm`) and `packages/viewer` (Vite + React + TS). `wasm-pack build` wired into the dev build.
- React shell: `<PresentationViewer src>` mounts, instantiates the WASM module, shows a blank canvas + a "loaded N bytes" readout.
- **Spike A — text backend.** Render one shaped line "Hello" two ways: (a) Canvas2D via a JS `measureText`/`fillText` bridge, (b) `cosmic-text` — rasterized in Rust — blitted. Record WASM size delta, visual fidelity, and cross-browser metric stability. **Write the decision into CLAUDE.md.** Default recommendation: Canvas2D bridge for accuracy speed, `cosmic-text` behind a feature flag for the determinism path.
- **Spike B — renderer trait.** Define `Renderer` + the `DisplayList` type. Stub the `webgpu` backend; implement `canvas2d`.
- Test harness: `fixtures/gen.py` (python-pptx) produces a blank + a one-textbox deck; `soffice --headless --convert-to png` renders goldens; `pixelmatch` diffs; `npm run test:golden` ties it together.

**Definition of done:** `npm run dev` shows the React component with a live WASM canvas; `npm run test:golden` runs green on a blank slide; CLAUDE.md records the text-backend decision.

**Verify:** `cargo test --workspace && npm run test:golden`

---

## Milestone 1 — Thin vertical slice  *(original Phase 1)*

**Goal:** open a real, trivial PPTX and render it end-to-end with navigation.

Tasks
- ZIP reader (`zip`) + `[Content_Types].xml` and `.rels` relationship resolution.
- Parse `presentation.xml` → ordered slide id list; parse one slide's `spTree`.
- Presentation Model: Slide, Shape, TextBox, Geometry — all in EMUs.
- Layout engine v0: position a text box and an `<a:prstGeom prst="rect">`, emit a DisplayList.
- Viewport: EMU→px, fit-to-screen, device-pixel-ratio handling.
- Navigation: next/prev slide, keyboard + API.

**Definition of done:** a 2-slide fixture (one text box + one rectangle per slide) opens, navigates both ways, and each slide is within tolerance of its LibreOffice golden.

**Verify:** `npm run test:golden -- --suite=m1`  ·  then `/clear` before M2.

---

## Milestone 2 — Text  *(original Phase 2: text)*

**Goal:** business text renders correctly.

Tasks: paragraphs & runs, horizontal/vertical alignment, word wrap, line spacing,
bold/italic/underline, font resolution + fallback chain, embedded fonts (`fntdata`),
bullets/numbering basics.

**Definition of done:** text-heavy fixture (mixed alignment, wrapping, weights, a
non-system font) passes within tolerance. Wrapping breakpoints match the oracle.

**Verify:** `npm run test:golden -- --suite=m2`

---

## Milestone 3 — Shapes & images  *(original Phase 2: rest)*

**Goal:** the common DrawingML vocabulary.

Tasks: preset geometries (rect, ellipse, roundRect, triangle, arrows…), lines &
connectors, custom geometry paths, group shapes with nested transforms (rotation/flip),
solid/line fills; images PNG/JPEG/SVG with source-crop (`srcRect`) and positioning.

**Definition of done:** shapes fixture and an images fixture (including a cropped image and
a rotated group) pass within tolerance.

**Verify:** `npm run test:golden -- --suite=m3`

---

## Milestone 4 — Masters, layouts, theme  *(original Phase 3)* — biggest accuracy jump

**Goal:** inheritance, which is where "corporate template" fidelity actually comes from.

Tasks: parse slide masters + layouts; implement the full resolution chain
(shape → placeholder → layout → master → theme); theme color scheme (incl. `phClr`
tint/shade), theme fonts (major/minor), placeholder matching by type+idx, slide/layout/master
backgrounds.

**Definition of done:** a real corporate-template fixture (title slide + content slide off a
shared master) matches within tolerance without per-shape overrides.

**Verify:** `npm run test:golden -- --suite=m4`  ·  then `/clear`.

---

## Milestone 5 — Tables, charts, effects  *(original Phase 4)* — split this one

**Goal:** the last big content categories. This is genuinely 3 milestones; sequence them.

- **5a Tables:** grid layout, cell fills/borders, merged cells (`gridSpan`/`rowSpan`), in-cell text.
- **5b Charts:** parse embedded `chart.xml`; bar, line, pie first; axes, legends, theme styling. (Charts are their own parser — budget accordingly.)
- **5c Effects:** gradient fills, transparency/alpha, outer shadow, glow, soft edges.

**Definition of done per sub-milestone:** its fixture passes; effects allowed a looser
tolerance since the oracle rasterizes them differently.

**Verify:** `npm run test:golden -- --suite=m5a` (then m5b, m5c)

---

## Milestone 6 — Performance  *(original Phase 5)*

**Goal:** hit the interaction targets without changing output.

Tasks: lazy per-slide parsing (parse on demand, prefetch neighbors), slide render cache,
move parsing/layout to a Web Worker, GPU texture/glyph-atlas cache (webgpu backend),
incremental load for large decks.

**Definition of done:** first slide renders fast on a large fixture; navigation and zoom hold
~60fps; golden tests still green (perf work must not alter pixels).

**Verify:** `npm run test:golden` (unchanged) + a bench script reporting first-slide ms and interaction fps.

---

## Milestone 7 — React SDK  *(original Phase 6)*

**Goal:** the shippable package.

Tasks: finalize the public API and props (`src`, `width`, `height`, `initialSlide`,
`onLoad`, `onError`, `onSlideChange`, `fit`), TS types, tree-shakeable build, docs, runnable
examples, API reference, and a chosen WASM-loading strategy (bundled vs. CDN).

Target API:
```tsx
<PresentationViewer src="/file.pptx" width="100%" height="100vh" />
```

**Definition of done:** `npm pack` produces a consumable package; a fresh example app renders
a deck with only the documented API; types resolve; works in Chrome, Edge, Firefox, Safari.

---

## How to actually drive Claude Code through this

- **Plan Mode for each milestone.** Let it propose the task breakdown, approve it, then let it execute. Don't hand it a whole milestone as one prompt.
- **One commit per task.** Small commits keep the golden diff attributable and make reverts cheap.
- **`/clear` between milestones** (marked above). Long single sessions degrade; the CLAUDE.md + PLAN.md carry the context forward, not the chat history.
- **Consider two subagents** for the parser (OOXML correctness) and the renderer (pixel fidelity) — they have different failure modes and test signals.
- **Golden tests are the contract.** If the agent proposes "temporarily skip the failing golden," say no; either fix it or widen the documented tolerance with a reason.
- **Guard the invariants.** If a diff shows layout code reaching into a canvas, or a renderer backend that only passes because it hardcoded a fixture, stop and correct — those break the abstraction the whole plan rests on.

## Where this will get hard (set expectations)
- **Text metrics** are 80% of perceived accuracy and the deepest rabbit hole — font matching, fallback, and wrap points. M2 will take longer than it looks.
- **Theme/placeholder inheritance (M4)** is the difference between "renders" and "looks right." Budget for it.
- **The oracle lies a little.** LibreOffice isn't PowerPoint; some diffs are the oracle's fault, not the viewer's. Keep a small set of PowerPoint-exported reference PNGs for manual spot-checks on the milestones that matter (M2, M4).
- **WebGPU text** (Backend B) is optional to shipping. If it stalls, ship on Canvas2D and revisit — the abstraction is there precisely so this decision stays cheap.
