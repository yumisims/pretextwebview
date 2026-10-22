# pretextview-egui

A contact-map viewer for [`.pretext`](https://github.com/sanger-tol/PretextMap) Hi-C files, written with [egui](https://github.com/emilk/egui). It follows the layout of [PretextView](https://github.com/sanger-tol/PretextView), the Sanger C++/OpenGL curation tool, and runs as a desktop window or in the browser.

[PretextView](https://github.com/sanger-tol/PretextView) remains the original application. This project is a separate viewer. It does not replace that program, and it does not reproduce its full behaviour.

`.pretext` files are not in this repository. They are large, and GitHub rejects files over 100MB.

## What works

- Opening a `.pretext` file from **Load Map**, or by dropping it on the view. The desktop app also opens `src/idOchAnnu3_RC-5104_normal.alg.pretext` at startup when that file is present. Otherwise it shows a small demo map.
- Drawing the contact map from the file's BC4 tiles, including an overview and finer tiles as you zoom in.
- Graph tracks stored after the tiles (coverage, gaps, repeats, telomeres), when the file contains them.
- Pan with the right mouse button, zoom with the scroll wheel, and gamma and colour-map controls.
- Editing the order of the map: drag across contigs to pick up a block, click outside it to move it, and **Invert** to flip it. **Esc** hides the options panel.
- Waypoints, scaffold labels, meta-data tags, a JSON state file, and AGP import and export.

## What does not match PretextView

- **Pixel Sort** does not read contact strength from the Hi-C image. It reorders fragments with a stand-in score, so the result is not a PretextView sort.
- **Pixel Cut** looks for break scores that exist only on the demo map. A loaded `.pretext` file has none, so Pixel Cut does not cut it.
- The colour-map names match PretextView's menu. The ramps themselves are short stand-ins, not PretextView's original 256-colour tables.
- Edits change the view, the JSON state, and the AGP text. They do not write a new `.pretext` file.
- Contigs shorter than one pixel of the full-resolution map are left out of the sequence list.
- The browser keeps the file, an overview of at most 2048 pixels on a side, and a small cache of finer tiles. It does not hold the multi-gigabyte working set PretextView builds. A very large map can fail to open in the browser.

## Desktop

Requires Rust.

```bash
cargo run --release
```

The binary is `target/release/pretextview-egui`.

## Browser

Requires the `wasm32-unknown-unknown` target and [trunk](https://trunkrs.dev/).

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
trunk serve --release --address 127.0.0.1 --port 8080
```

Open http://127.0.0.1:8080. `trunk build --release` writes the site to `dist/` without serving it.

`index.html` copies `src/idOchAnnu3_RC-5104_normal.alg.pretext` into the site when that file is present. Without it, remove that `copy-file` line before building.
