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
| `npm run build` | build the publishable package |
| `cargo test --workspace` | Rust tests — parsing, layout, culling, renderer behaviour |
| `npm run test:golden` | render every fixture and pixel-diff against LibreOffice |
| `npm run test:golden -- --suite=m2` | one suite |
| `npm run test:browsers` | render every fixture in Chromium, Firefox and WebKit |
| `npm run bench` | performance numbers |
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
| `fixtures/` | `gen.py`, which builds every fixture the golden suite uses |
| `tests/golden/` | the oracle, the runner, the bench, the cross-browser check |

## Licence

MIT
