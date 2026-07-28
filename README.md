# pptx-viewer

A browser-native, read-only `.pptx` renderer. Rust compiled to WebAssembly does the
parsing, layout and drawing; TypeScript wraps it; React is optional. Nothing is uploaded
and nothing is converted server-side.

For using the package, see [`packages/viewer/README.md`](packages/viewer/README.md).
This file is about working on it.

```tsx
<PresentationViewer src="/deck.pptx" width="100%" height="100vh" />
```

## How it fits together

```
.pptx bytes
   ↓  crates/core::opc        ZIP container, [Content_Types].xml, the .rels graph
   ↓  crates/core::parse      OOXML → presentation model (still in EMUs, still unresolved)
   ↓  crates/core::layout     inheritance, text measurement, geometry → display list (points)
   ↓  crates/renderer         Renderer trait → canvas2d | record | webgpu
   ↓  crates/wasm             wasm-bindgen surface
   ↓  packages/viewer         async work: fetching, image decoding, font loading, React
canvas
```

Two conventions carry a lot of weight, and both are explained in
[`CLAUDE.md`](CLAUDE.md):

- **The display list is in points, not pixels.** Zoom, resize and DPR changes re-render
  without re-laying-out, which is why wrap points cannot drift between zoom levels.
- **`None` means "not specified", not "default".** The whole inheritance chain depends on
  telling an unset property from one explicitly set to a default value.

`CLAUDE.md` also records the decisions that shaped the design — the text backend, the
renderer abstraction, and why there is no Web Worker — each with the measurements behind
it.

## How it compares

Against the six other browser-side pptx renderers on npm, over the 11 fixtures every
engine renders. Payload is gzipped and measured by bundling each engine with esbuild;
`cold` is a first render in a fresh page including module and WASM instantiation; `warm`
is opening another deck in the same page; `content` is the share of *inked* pixels
differing from the LibreOffice render.

| engine | payload | cold | warm | content |
|---|---:|---:|---:|---:|
| [@jvmr/pptx-to-html](https://www.npmjs.com/package/@jvmr/pptx-to-html) 1.1.1 | 45 KB | 20.6 ms | 2.9 ms | 61.33% |
| [pptx-glimpse](https://www.npmjs.com/package/pptx-glimpse) 5.0.0 | 167 KB | 38.3 ms | 9.4 ms | 25.39% |
| [pptxviewjs](https://www.npmjs.com/package/pptxviewjs) 1.1.9 | 252 KB | 102.8 ms | 38.9 ms | 23.05% |
| **pptx-viewer** (this project) | **319 KB** | **29.2 ms** | **2.0 ms** | **20.53%** |
| [@aiden0z/pptx-renderer](https://www.npmjs.com/package/@aiden0z/pptx-renderer) 1.2.4 | 349 KB | 94.1 ms | 5.9 ms | 20.42% |
| [pptx-preview](https://www.npmjs.com/package/pptx-preview) 1.0.7 | 426 KB | 93.1 ms | 5.2 ms | 28.59% |
| [pptx-vanilla-viewer](https://www.npmjs.com/package/pptx-vanilla-viewer) 1.6.2 | 1695 KB | 223.6 ms | 27.4 ms | 63.17% |

`npm run compare` reproduces this; `-- --file=deck.pptx` scores your own deck.

**Where this project leads.** Speed against the engines that render accurately: 3.2×
faster cold and 3.0× faster warm than @aiden0z/pptx-renderer, the only one whose fidelity
matches. (@jvmr/pptx-to-html starts faster still, at 20.6 ms — its content figure says
what that buys.) And *structural* coverage, which is what the content figure is actually
good at detecting:

| | ours | @aiden0z | pptx-preview | pptx-glimpse | vanilla |
|---|---:|---:|---:|---:|---:|
| tables (`m5a`) | **1.65%** | 2.16% | 62.07% | 62.20% | 43.31% |
| preset shapes (`m3`) | **0.66%** | 1.43% | 4.71% | 5.78% | 72.01% |

**Where it does not.** It is *not* the smallest — fourth of seven. And on fidelity it is
tied with @aiden0z/pptx-renderer (20.53% vs 20.42%), not ahead; that engine is the real
competition, at 10% more payload and 3.2× the cold time.

**What the content figure cannot tell you.** On the text-only fixture every engine scores
46–49%, because that is measuring font rasterisation against LibreOffice rather than
correctness. Read it on structural fixtures, not textual ones. And on the tiled-fill
fixture the oracle is simply wrong — engines that stretch the tile, as it does, score
*better* than engines that repeat it correctly. See the oracle discussion under
[Testing](#testing) before drawing conclusions from any single row.

Two of these packages do not install cleanly: `pptxviewjs` imports `chart.js/auto` and
`pptx-vanilla-viewer` imports `three`, neither declared as a dependency. Both are
installed explicitly in `examples/comparison` and counted in their payload.

## Getting set up

```sh
# Rust, for the core
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown
brew install wasm-pack binaryen        # or your platform's equivalent

# Node
npm install

# Fixture generation and the golden-test oracle
python3 -m venv .venv && ./.venv/bin/pip install python-pptx Pillow
brew install --cask libreoffice
brew install poppler
npx playwright install chromium firefox webkit
```

`npm run test:golden` reports which of these are missing and degrades rather than failing
outright, so you can work on the Rust without the whole toolchain installed.

## Commands

| | |
|---|---|
| `npm run dev` | build the WASM and serve the dev app on :5178 |
| `npm run wasm` | build the WASM only |
| `npm run build:pkg` | build the WASM **and** the package — what the examples consume |
| `npm run build` | build the publishable package |
| `cargo test --workspace` | Rust tests — parsing, layout, culling, renderer behaviour |
| `npm run test:golden` | render every fixture and pixel-diff against LibreOffice |
| `npm run test:golden -- --suite=m2` | one suite |
| `npm run test:browsers` | render every fixture in Chromium, Firefox and WebKit |
| `npm run bench` | performance numbers for this renderer alone |
| `npm run compare` | benchmark against the six other npm pptx renderers |
| `npm run fixtures` | regenerate the `.pptx` fixtures |
| `npm test` | all three test layers in sequence |

The browser-driven commands start a dev server if one is not already up, and leave a
server you started yourself alone.

## Testing

Three layers, each answering a different question.

**`cargo test`** asks *is the logic right?* Layout runs against a synthetic measurer
(`StubMeasure`), so it asserts where line breaks fall without depending on which fonts the
machine has. The recording renderer lets renderer behaviour — clip nesting, transform
composition, what gets culled — be asserted with no browser involved.

**`npm run test:golden`** asks *do the pixels match?* Fixtures are generated with
python-pptx, rendered by headless LibreOffice via PDF, rendered by the viewer in headless
Chromium, and diffed. Per-suite tolerances live in
[`tests/golden/suites.json`](tests/golden/suites.json), each with a written reason.

The oracle is LibreOffice, not PowerPoint. It is *consistent*, which is what a regression
detector needs, but some diffs are its fault. Two things follow from that: a tolerance is
a decision to record rather than a knob to turn, and pixel tolerance is a poor detector of
"did this feature stop working" — a shadow vanishing entirely moves the figure by half a
percent. Unit tests are the guard for that.

Sometimes the oracle is not merely imprecise but *wrong*: it ignores `<a:tile>` on a blip
fill and stretches the image instead. Diffing against it there would score a correct render
worse than a broken one. Such a suite sets `"oracle": false` and compares against a
reviewed reference committed in [`tests/golden/reference/`](tests/golden/reference/); the
runner labels those lines so the weaker guarantee is visible where the number is read. So
before trusting a new tolerance, check the oracle renders the feature at all.

**`npm run test:browsers`** asks *does it work everywhere?* Every fixture is rendered in
all three engines and the extracted text compared. This is where the text-backend decision
gets checked against reality rather than argued about.

### When a golden diff appears

Get `debugTrace()` for the slide first — the runner writes one per slide to
`tests/golden/out/trace/`. It tells you in one look whether layout moved something or the
rasteriser drew it differently, which is the most common ambiguity in this project.

## Repository layout

| | |
|---|---|
| `crates/core` | OPC container, OOXML parsers, presentation model, layout, display list |
| `crates/renderer` | the `Renderer` trait, `canvas2d`, `record`, `webgpu` (a measured stub), culling |
| `crates/wasm` | the wasm-bindgen surface — deliberately narrow |
| `packages/viewer` | the TypeScript API and the React component |
| `examples/basic` | a small app using only the documented API |
| `examples/comparison` | every engine side by side on the same deck, including your own |
| `fixtures/` | `gen.py`, which builds every fixture the golden suite uses |
| `tests/golden/` | the oracle, the runner, the bench, the cross-browser check |

## Licence

MIT
