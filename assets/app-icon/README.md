# Terminal Gremlin

Selected Griz app icon: lime terminal gremlin on a purple tile, generated from
Daniel's chosen concept. `terminal-gremlin.png` is embedded by the app for the
Dock and app switcher, including development launches. Transparent margins are
preserved.

`Griz.icns` is the macOS bundle asset. Regenerate it with:

```sh
sh scripts/build-icon.sh
```

`scripts/package-macos.sh` copies `Griz.icns` into `Contents/Resources` and sets
`CFBundleIconFile` to `Griz`. The development launcher runs the binary directly
and uses Rune's runtime app-icon API.
