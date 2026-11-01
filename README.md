# Spellbook — the file shelf ✨

```
  ____
 |    |___
 |        |
 |________|
```

Pink file shelf for faeOS. Rust engine, event-driven redraw (no 10 Hz Python paint).

For: browsing home, yank/paste, `--pick` for Tome and Fairy Lantern. Not for: becoming Midnight Commander.

```
git clone git@github.com:ElegantVW/spellbook.git ~/spellbook
cd ~/spellbook && ./build.sh install --house
spellbook ~
```

## Look

`j/k` move · `enter` / `l` / `o` open · `h` parent · `:` / `Ctrl-L` path · `i` overview · `y` yank · `Y` yank shelf · `P` paste-go (`imbue`) · `.` hidden · `s` sort · `n` new (name ending `/` is a dir) · `d` delete · `r` rename · `e` editor · `?` help · `q` quit.

Rows tint by kind (dir / link / exec / image / video / audio / archive / Markdown / markup / PDF / code / text). Click to select, double-click to open, wheel to move. Selected is a light `▸`, not a reverse bar.

The **Overview** pane shows the selected entry's type, size, modified time, permissions, path, and a read-only preview. Text is capped at 4 KiB; Markdown headings, lists, quotes, inline emphasis/code, and fenced code are styled, while HTML/XML tags and attributes use distinct colors. PDF previews extract text from the first page with `pdftotext`. Raster images are decoded and scaled into terminal cells; SVG is shown as markup text. Video previews show an extracted frame using `ffmpeg`. With a video selected and Overview open, `v` starts/pauses playback, `m` toggles mute, and `+`/`-` adjust volume. Playback uses `mpv` controlled through its local IPC socket and is stopped when the Overview closes, selection changes, or Spellbook exits. `ffmpeg`, `mpv`, and `pdftotext` are optional; Spellbook reports when a required tool is unavailable. Symbolic links, directories, and special files are not previewed. On narrow terminals the pane uses the listing area; press `i` again to return to the file list.

Pick: `spellbook --pick --output /tmp/out [dir]`.

## Layout

| Path | Role |
|------|------|
| `src/` | engine |
| `scripts/spellbook` | thin launcher |
| `./build.sh install --house` | ELF → `~/.local/lib/faeos/spellbook` |

Never commit a prebuilt ELF.

## License

MIT — see [LICENSE](LICENSE).
