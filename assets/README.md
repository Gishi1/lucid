# Visual assets

| File | Size | Purpose |
| --- | --- | --- |
| `banner.png` | 2400×600 | README banner |
| `icon.png` | 512×512 | Application mark / repository avatar |
| `social-preview.png` | 1280×640 | GitHub social preview (repository settings) |
| `screenshots/reading-dark.png` | 1280×800 | Reader, dark theme |
| `screenshots/reading-light.png` | 1280×800 | Reader, light theme |
| `screenshots/search.png` | 1280×800 | Search pane |

The palette matches `crates/lucid/src/theme.rs`: accent `#4F8CFF`, secondary `#34D399`,
chrome background `#111318` / `#181B21`. Prefer those colours when replacing an asset.

## Regenerating screenshots

Screenshots are real captures of the committed demo package, not mockups. Build the GUI and
run:

```bash
cargo build
./target/debug/lucid testdata/demo-help.mshc --topic Lucid.Reference.Markup --size 1280x800 \
    --frames 20 --screenshot assets/screenshots/reading-dark.png
./target/debug/lucid testdata/demo-help.mshc --topic Lucid.Reference.Markup --size 1280x800 \
    --theme light --frames 20 --screenshot assets/screenshots/reading-light.png
./target/debug/lucid testdata/demo-help.mshc --topic Lucid.Guides.Search --pane search \
    --search index --size 1280x800 --frames 60 --screenshot assets/screenshots/search.png
```

Keep captures at 1280×800, use only `testdata/demo-help.mshc` (never vendor packages), and
check that no window decoration or desktop background is included — `--screenshot` captures
the egui surface only.
