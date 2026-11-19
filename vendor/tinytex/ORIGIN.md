# Vendored TinyTeX (for `cmx stex compile`)

cmx ships a small TeX distribution so `cmx stex compile` needs no MacTeX/TeX Live install
(user decision, session 3; STEX-DESIGN §1). At first use cmx unpacks the archive for the current
platform next to it (`vendor/tinytex/<platform>/`, git-ignored) and runs `pdflatex` / `xelatex` /
`lualatex` from there.

| File | Platform | sha256 |
|---|---|---|
| `TinyTeX-1-darwin-v2026.11-cmx1.tar.xz` | macOS (universal: arm64 + x86_64) | `3bfdeb54308531ad667e81791d191c661dfc0f9584feaf2d8865713fa3457e62` |
| `TinyTeX-1-linux-x86_64-v2026.11-cmx1.tar.xz` | Linux x86_64 (glibc) | `229f275bfbf6dbfffd0eb440d9d4bae541bad9d68fd79dcf3bde9e5591acaecd` |
| `TinyTeX-1-linux-arm64-v2026.11-cmx1.tar.xz` | Linux aarch64 (glibc) | `cbea580c2e41fac3e756afefa5b0cb17ee0c333f76e6cf447134d104ff76c599` |

## Origin

TinyTeX release **v2026.11** (2026-11-01), bundle "TinyTeX-1", from
https://github.com/rstudio/tinytex-releases/releases/tag/v2026.11

| Upstream asset | sha256 |
|---|---|
| `TinyTeX-1-darwin-v2026.11.tar.xz` | `9416a257c4caea2f82351fa2f89252b8bd75dceeb0ce6e442d25530b1922a02d` |
| `TinyTeX-1-linux-x86_64-v2026.11.tar.xz` | `5b75befa848deae702ae6fa12ff0eef3f110d7e4e2a979ae99f101eba3c9c70d` |
| `TinyTeX-1-linux-arm64-v2026.11.tar.xz` | `b351c365e4085a179c8d76fddadff612d4e4fd16a946953fb8b9d331f4f7344a` |

## cmx modifications ("cmx1")

1. **Added packages** that stex output can use but TinyTeX-1 lacks: `needspace`, `jknapltx`
   (`mathrsfs.sty`), `rsfs` (fonts), `enumitem`, `pgf` (TikZ) and its dependency `fp`. They were
   installed with TinyTeX's own `tlmgr` (after `tlmgr update --self`) on macOS, from the TinyTeX
   CTAN repository (`https://tlnet.yihui.org`), on 2026-11-13.
2. **Linux trees:** the 546 new `texmf-dist` files and 6 `tlpkg/tlpobj` files were copied from the
   macOS tree. `texmf-dist` is platform-independent, and the three upstream trees are byte-identical
   outside `bin/` except for generated files. `MixedMap rsfs.map` was appended to `updmap.cfg`, the
   regenerated font maps (`texmf-var/fonts/map`) were copied, the new packages' records were appended
   to `tlpkg/texlive.tlpdb`, and `texmf-dist/ls-R` was regenerated. The `ls-R` generator was checked
   by swapping its output into the macOS tree and compiling. The Linux binaries themselves were
   **not** run (no Linux machine available); the Linux formats are upstream's.
3. **Layout:** the Linux upstream archives unpack to `.TinyTeX/`; repacked as `TinyTeX/` like macOS.
4. Removed `tlmgr` logs and caches (`texmf-var/web2c/tlmgr*.log`, `updmap.log`,
   `tlpkg/texlive.tlpdb.main.*`, `tlpkg/backups`).

Verified on macOS 27.2 arm64: `pdflatex`, `xelatex` and `lualatex` compile a document that uses every
package stex can emit (amsmath, amssymb, mathrsfs, needspace, enumitem, tikz, geometry, lmodern), with
no warnings.

## Licenses

- TinyTeX (the installer scripts and bundle definitions): MIT, © 2017–2026 Yihui Xie and Posit
  Software, PBC.
- Contents are TeX Live, whose components carry their own free-software licenses: mostly the LaTeX
  Project Public License (LPPL), GPL, the Knuth license for TeX/METAFONT, and the SIL OFL / GUST
  Font License for fonts. Per-package license data is in `tlpkg/texlive.tlpdb` (the `catalogue-license`
  fields) inside each archive. All of it allows redistribution.

## Updating

Download the new TinyTeX-1 assets and repeat the steps above, then update the version constant in
`crates/cmx/src/tinytex.rs` and this file.
