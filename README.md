# Spellbook — terminal file manager ✨

Pink file shelf for faeOS. Rust engine, event-driven redraw (no 10 Hz Python paint).

```
git clone git@github.com:ElegantVW/spellbook.git ~/spellbook
cd ~/spellbook && ./build.sh install --house
spellbook ~
```

## Keys

`j/k` move · `enter` / `l` / `o` open · `h` parent · `:` / `Ctrl-L` path · `y` yank · `Y` yank shelf · `P` paste-go (`imbue`) · `.` hidden · `s` sort · `n` new (name ending `/` is a dir) · `d` delete · `r` rename · `e` editor · `?` help · `q` quit.

Rows tint by kind (dir / link / exec / image / audio / archive / code). Click to select, double-click to open, wheel to move. Selected is a light `▸`, not a reverse bar.

Pick (Tome, Fairy Lantern): `spellbook --pick --output /tmp/out [dir]`.

## Layout

| Path | Role |
|------|------|
| `src/` | engine |
| `scripts/spellbook` | thin launcher |
| `./build.sh install --house` | ELF → `~/.local/lib/faeos/spellbook` |

Never commit a prebuilt ELF.

## License

MIT — see [LICENSE](LICENSE).
