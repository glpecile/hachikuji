# Releasing

A release is a GitHub release with prebuilt archives, plus a formula in the Homebrew tap. Users get it in three ways:

| Install method | Reads | Gets a new version with |
|---|---|---|
| Script | `releases/latest/download/install.sh` | `hachikuji update` |
| Homebrew | `Formula/hachikuji.rb` in [glpecile/homebrew-tap](https://github.com/glpecile/homebrew-tap) | `brew upgrade hachikuji` |
| Source | this repository | `git pull && make install` |

## Version numbers

[Semantic versioning](https://semver.org), from the commit types since the last tag: a `feat` commit is a minor release (1.1.0), only `fix` commits are a patch release (1.0.1), and a breaking change is a major release (2.0.0). Breaking changes include a change to the config keys, to the `.zshrc` line, or to the archive layout that `install.sh` reads.

## Steps

You need `gh`, `rustup` and Docker (OrbStack works; its `docker` is in `/Applications/OrbStack.app/Contents/MacOS/xbin`). Without Docker, `make release` skips the Linux archive and prints no error.

1. **Bump the version.** Change `version` in `Cargo.toml`, run `cargo build` (it updates `Cargo.lock`), and open a PR titled `chore: release X.Y.Z`. Merge it when CI passes.
2. **Build.** On an up-to-date `main`, run `make release`. It writes to `build/release/`:
   - `hachikuji-X.Y.Z-{aarch64-apple-darwin,x86_64-apple-darwin,aarch64-unknown-linux-gnu}.tar.gz`
   - `install.sh`, with the version in it
   - `hachikuji.rb`, the Homebrew formula with the URLs and SHA-256 checksums of the two macOS archives
3. **Check the archives** before you publish. This installs from disk through the new `install.sh`:
   ```sh
   p=$(mktemp -d)
   HACHIKUJI_URL=file://$PWD/build/release PREFIX=$p sh build/release/install.sh
   $p/bin/hachikuji version          # X.Y.Z
   HACHIKUJI_INDEX=$p/share/hachikuji/specs.idx $p/bin/hachikuji complete 'git chec'   # count=1 ...
   ```
4. **Publish.** This creates the tag `vX.Y.Z` on `main` and makes the release "Latest":
   ```sh
   gh release create vX.Y.Z --generate-notes build/release/*.tar.gz build/release/install.sh
   ```
5. **Update the tap.** Copy `build/release/hachikuji.rb` to `Formula/hachikuji.rb` in a clone of [glpecile/homebrew-tap](https://github.com/glpecile/homebrew-tap), commit it as `feat: hachikuji X.Y.Z`, and push. Do this after step 4: the formula points at the release archives.
6. **Check what users get:**
   ```sh
   curl -fsSL https://github.com/glpecile/hachikuji/releases/latest/download/install.sh | PREFIX=$(mktemp -d) sh
   brew update && brew upgrade hachikuji     # or, on a machine without it: brew install glpecile/tap/hachikuji
   ```

For about 30 seconds after step 4, GitHub can still redirect `releases/latest` to the old tag. In that time, `hachikuji update` says "up to date" or installs the old version. Wait, then check again.

## The Homebrew tap

[glpecile/homebrew-tap](https://github.com/glpecile/homebrew-tap) is a Git repository that holds one file, `Formula/hachikuji.rb`. Homebrew clones a tap to `$(brew --repository)/Library/Taps/glpecile/homebrew-tap` and reads the formulas in it. The `homebrew-` prefix lets users write `glpecile/tap` for short.

### What the formula does

It downloads the release archive for the Mac's CPU (arm64 or x86_64), checks its SHA-256, and installs:

- `bin/hachikuji` to `$(brew --prefix)/bin/hachikuji`
- `hachikuji.zsh`, `specs.idx` and `FIG-LICENSE` to `$(brew --prefix)/share/hachikuji/`

It does not build from source, so users do not need Rust or Bun. `brew test` runs `hachikuji init zsh` and `hachikuji complete 'git chec'`. The template is `packaging/homebrew/hachikuji.rb.in`, and `make release` fills in the version, the URLs and the checksums. Change the template, not the file in the tap: the next release overwrites the tap file.

The binary finds its zsh file through the `bin` and `share` symlinks in `$(brew --prefix)`, not through the versioned folder in `Cellar/`. So the line in `.zshrc` stays valid after `brew upgrade`.

### Why a tap, and not homebrew-core

homebrew-core does not accept hachikuji now. Its [acceptance policy](https://docs.brew.sh/Package-Acceptance-Policy) asks for at least 30 forks, 30 watchers or 75 stars (90, 90 or 225 when the owner submits it), and a repository at least 30 days old. homebrew-core also builds each formula from source on its own CI. A tap has none of these rules, and we control when a new version ships.

The cost is one manual step per release (step 5). Move to homebrew-core when the project meets the policy. Then delete the tap, or leave a formula that says where it moved.

### Tap trust

Homebrew 7 does not load formulas from a tap that the user has not trusted. `brew install glpecile/tap/hachikuji` taps the repository and trusts that one formula, in one command. `brew upgrade` then works.

So the docs give only that command. `brew tap glpecile/tap` followed by `brew install hachikuji` fails with "Refusing to load formula glpecile/tap/hachikuji from untrusted tap". To fix that, run `brew trust --formula glpecile/tap/hachikuji`. `brew untap` removes the trust too.

### Limits

- macOS only. The formula has no Linux URL, so Homebrew on Linux cannot install it. Linux users use the script.
- `hachikuji update` does not update a Homebrew install. It sees `Cellar` in the binary's real path and prints `brew upgrade hachikuji`, because files that change outside Homebrew break `brew upgrade` and `brew uninstall`.
