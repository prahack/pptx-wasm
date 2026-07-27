# Renderer comparison

Puts this viewer next to other browser-side pptx renderers on the same file.

```sh
npm install
npm run dev          # http://localhost:5179 — side by side, pick a fixture
```

For measured numbers rather than an eyeball, run the harness from the repo root:

```sh
npm run compare
npm run compare -- --suite=m5a --runs=5
```

It loads each engine in its own page (so neither warms the other's caches), records cold
and warm open+render times, screenshots the result, and diffs it against the same
LibreOffice render the golden suite uses — so both engines are judged by an implementation
neither of them wrote.

Renders and diffs land in `tests/golden/out/compare/`.

## Adding another engine

Add an entry to `ENGINES` in `src/main.tsx` with a `run(host, bytes, slide)` that resolves
once the slide is drawn, and to `ENGINES` in `tests/golden/compare.mjs`. The rules that
keep it fair: same file, same slide, same pixel size, and stop the clock only when the
engine says it has finished — not when its promise resolves.
