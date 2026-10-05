# Regenerating the syntax-highlighting asset

`assets/highlight/syntaxes.syntect` is a committed binary: a pre-dumped,
curated syntect `SyntaxSet` covering the ~30 languages in
`docs/listings.md`. texforge ships it via `include_bytes!` — no yaml/plist
parsing and no network at build time or at runtime.

Regenerate it only when the language manifest changes or the workspace
`syntect` pin moves (the dump format is version-marked; the `=5.3.0` pin
here and the runtime `5.3` pin move together, with a regen in the same
commit series).

```bash
git clone --depth 1 https://github.com/sublimehq/Packages vendor/Packages
cargo run --release --manifest-path scripts/regen-syntaxes/Cargo.toml -- \
  --source ./vendor/Packages \
  --out ./assets/highlight/syntaxes.syntect
```

Network happens ONLY in the `git clone` above, run by hand by a maintainer —
never during `cargo build` of texforge and never at runtime.

## If the binary-size budget breaks

The release-size gate is `stripped size − baseline ≤ 2 MiB`. If a regen
pushes the build over budget, drop languages in this order and regen
(Scala is already excluded from the manifest, so it is listed first as a
reminder never to re-add it casually):

Scala, OCaml, Clojure, Erlang, Haskell, PHP, Java, Matlab, Graphviz, Diff,
Git, Makefile.

Never drop: Python, Rust, JavaScript, Bash, JSON, YAML, TOML, Markdown,
LaTeX, R, C, C++, SQL.
