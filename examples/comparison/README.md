# Renderer comparison

Puts this viewer next to other browser-side pptx renderers on the same file.

```sh
npm run build:pkg    # from the repo root — builds the WASM *and* the package
cd examples/comparison && npm install
npm run dev          # http://localhost:5179 — side by side, pick a fixture
```

`npm run build:pkg` matters. This app imports `pptx-wasm`, which resolves to
`packages/viewer/dist` — a copy made by the package build, not the WASM build. Running
only `npm run wasm` after a Rust change leaves it stale and the app silently runs the
previous build. The dev server refuses to start in that state rather than let you debug a
fix that is not loaded.

For measured numbers rather than an eyeball, run the harness from the repo root:

```sh
npm run compare
npm run compare -- --suite=m5a --runs=5
```

It loads each engine in its own page (so neither warms the other's caches), records cold
and warm open+render times, screenshots the result, and diffs it against the same
LibreOffice render the golden suite uses — so both engines are judged by an implementation
neither of them wrote.

Renders and diffs land in `tests/golden/out/compare/`, one PNG per engine per slide.

## Your own deck

Both paths take one:

```sh
npm run dev                                   # then use the file picker, or drag one in
npm run compare -- --file=~/decks/mine.pptx   # measured, scored against LibreOffice
```

Nothing is uploaded — both engines run in the page, so a confidential deck stays on your
machine.

## Reading the accuracy numbers

Two figures, because each on its own misleads.

**% of slide** is the conventional pixel-diff ratio. It understates text errors badly: a
slide is mostly white and glyphs are thin strokes, so a body-text block rendered at the
wrong size in the wrong place — obviously broken to a human — moves this figure by about
one percent.

**% of content** divides the same difference by the pixels carrying content in either
image. It catches those layout errors, but over-weights antialiasing: two renderers that
agree on every glyph's position but rasterise it differently still show a large number.

Neither is an absolute score. What is meaningful is the *comparison between engines on the
same fixture*, since both carry the same bias. A gap that shows up in both figures — the
table and effects fixtures, say — is a real feature difference. A gap that appears in only
one is probably an artifact of the metric.

Slides are scored individually and averaged. Scoring only slide 1 would flatter everyone,
since slide 1 is usually the title — the simplest slide in the deck.

## Adding another engine

Add an entry to `ENGINES` in `src/main.tsx` with a `run(host, bytes, slide)` that resolves
once the slide is drawn, and to `ENGINES` in `tests/golden/compare.mjs`. The rules that
keep it fair: same file, same slide, same pixel size, and stop the clock only when the
engine says it has finished — not when its promise resolves.
