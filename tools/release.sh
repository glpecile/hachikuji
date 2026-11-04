#!/bin/sh
# Builds the release archives and the Homebrew formula. Run with `make release`.
# Each archive: bin/hachikuji, share/hachikuji/{hachikuji.zsh,specs.idx,FIG-LICENSE}, README.md.
# It publishes nothing: upload with `gh release create v<version> build/release/*.tar.gz build/release/install.sh`.
set -eu
cd "$(dirname "$0")/.."
PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=build/release
rm -rf "$out"
mkdir -p "$out"

pack() { # pack <target> <binary>
  d="$out/hachikuji-$v-$1"
  mkdir -p "$d/bin" "$d/share/hachikuji"
  cp "$2" "$d/bin/hachikuji"
  cp shell/hachikuji.zsh build/specs.idx specs/FIG-LICENSE "$d/share/hachikuji/"
  cp README.md "$d/"
  tar -C "$out" -czf "$d.tar.gz" "hachikuji-$v-$1"
  rm -rf "$d"
}

for t in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target add "$t" >/dev/null 2>&1 || true
  cargo build --release --quiet --target "$t"
  pack "$t" "target/$t/release/hachikuji"
done
if docker version >/dev/null 2>&1; then
  docker run --rm -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/src/target/linux rust:1-slim \
    cargo build --release --quiet
  pack aarch64-unknown-linux-gnu target/linux/release/hachikuji
fi

sha() { shasum -a 256 "$out/hachikuji-$v-$1.tar.gz" | cut -d' ' -f1; }
url="https://github.com/glpecile/hachikuji/releases/download/v$v"
sed -e "s|@VERSION@|$v|g" -e "s|@URL@|$url|g" \
    -e "s|@SHA_ARM@|$(sha aarch64-apple-darwin)|" -e "s|@SHA_INTEL@|$(sha x86_64-apple-darwin)|" \
    packaging/homebrew/hachikuji.rb.in > "$out/hachikuji.rb"
sed "s|@VERSION@|$v|g" packaging/install.sh.in > "$out/install.sh"
ls -l "$out"
