# pretextwebview

A contact-map viewer for [`.pretext`](https://github.com/sanger-tol/PretextMap) Hi-C files, written with [egui](https://github.com/emilk/egui). It follows the layout of [PretextView](https://github.com/sanger-tol/PretextView), the Sanger C++/OpenGL curation tool, and runs as a desktop window or in the browser.

[PretextView](https://github.com/sanger-tol/PretextView) remains the original application. This project is a separate viewer. It does not replace that program, and it does not reproduce its full behaviour.

## What works

- Opening a `.pretext` file from the welcome screen's **Choose .pretext file** button or drop area, or from **Load Map** using an HTTPS link.
- Drawing the contact map from the file's BC4 tiles, including an overview and finer tiles as you zoom in.
- Pan with the right mouse button, zoom with the scroll wheel, and gamma and colour-map controls.
- Background colours, **Default Gamma**, and **Dark**, **Black**, **White**, **Red**, and **Blue** menu themes.
- Visibility controls for waypoints, scaffolds, contig name labels, scale bars, grid, ID bars, and tooltips.
- **Invert Mouse Buttons** switches panning to the left mouse button.
- The **Esc** button hides the menu; the **Esc** key hides or restores it. **N** toggles contig name labels and **T** toggles tooltips when no text field is focused.

The `static` branch exposes only these viewing controls. Editing, sorting, suggestions, extension tracks, state files, AGP import/export, and their interaction shortcuts are not available in this interface. Their underlying implementation is retained. Dropped JSON and AGP files are rejected rather than loaded as an alternate entry point.

## What does not match PretextView -- a lot yet

- The colour-map names match PretextView's menu. The ramps themselves are short stand-ins, not PretextView's original 256-colour tables.
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
