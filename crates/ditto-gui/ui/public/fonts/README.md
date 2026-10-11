# Vendored fonts

These are committed rather than loaded from a CDN. This tool reads your private
configuration files; it should not be making network requests while it does so,
and it should work on a machine with no connection at all.

Each file is the **latin subset only**, taken from the Google Fonts CSS API.

| File | Family | Weights |
|---|---|---|
| `Archivo-400_700.woff2` | [Archivo](https://fonts.google.com/specimen/Archivo) | 400–700 variable |
| `ArchivoNarrow-500_700.woff2` | [Archivo Narrow](https://fonts.google.com/specimen/Archivo+Narrow) | 500–700 variable |
| `IBMPlexMono-400.woff2` | [IBM Plex Mono](https://fonts.google.com/specimen/IBM+Plex+Mono) | 400 |
| `IBMPlexMono-500.woff2` | IBM Plex Mono | 500 |
| `IBMPlexMono-600.woff2` | IBM Plex Mono | 600 |

About 99 KB in total.

## Licence

All three families are licensed under the
[SIL Open Font License 1.1](https://openfontlicense.org/), which permits
bundling and redistribution. Archivo is by Omnibus-Type; IBM Plex Mono is by
IBM / Bold Monday.

The OFL requires that the fonts are not sold on their own and that any modified
version is not distributed under the original name. Neither applies here: these
are unmodified subsets shipped as part of an application.

## Why these faces

Archivo is drawn for dense, small, tabular settings — which is what the matrix
is. Archivo Narrow appears only in the five column headers, where width is the
binding constraint. IBM Plex Mono carries every path, config fragment and diff
line: monospace here is the material, not decoration, and its disambiguated
glyphs matter when you are reading `~/.codex` against `~/.config`.
