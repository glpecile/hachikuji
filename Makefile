# hachikuji. `make` builds the helper and the spec index; `make test` runs every check.
# Build-time tools: cargo, bun, git. Runtime: only the binary, the zsh file and the index.

FIG_REPO   := https://github.com/withfig/autocomplete
FIG_COMMIT := aef52acff84c45edde61ae610cc2c964802b9a38
BIN        := target/release/hachikuji
# Homebrew's rustup is keg-only: put it first so cargo finds rustc.
export PATH := /opt/homebrew/opt/rustup/bin:$(HOME)/.cargo/bin:$(PATH)
CARGO      := $(shell PATH="$(PATH)" command -v cargo)

.PHONY: all helper index install test unit integration bench memory e2e startup release clean
PREFIX ?= $(HOME)/.local
all: helper index

helper:
	$(CARGO) build --release

# The Fig specs at the pinned commit. They are parsed, never run (tools/extract.ts).
build/fig:
	git init -q $@
	git -C $@ fetch -q --depth 1 $(FIG_REPO) $(FIG_COMMIT)
	git -C $@ checkout -q FETCH_HEAD

build/tools/node_modules/typescript:
	mkdir -p build/tools
	echo '{"name":"hk-tools","private":true}' > build/tools/package.json
	cd build/tools && bun add typescript@5.5.4 --ignore-scripts

index: build/specs.idx
build/specs.idx: helper build/fig build/tools/node_modules/typescript tools/extract.ts
	rm -rf build/specs-json
	bun tools/extract.ts build/fig build/tools build/specs-json specs/lost.txt
	cp build/fig/LICENSE specs/FIG-LICENSE
	$(BIN) compile build/specs-json $@
	$(BIN) compile specs build/m1.idx
	@ls -l $@

# bin/hachikuji and share/hachikuji/{hachikuji.zsh,specs.idx,FIG-LICENSE} under PREFIX.
install: all
	install -d $(PREFIX)/bin $(PREFIX)/share/hachikuji
	install -m 755 $(BIN) $(PREFIX)/bin/hachikuji
	install -m 644 shell/hachikuji.zsh build/specs.idx specs/FIG-LICENSE $(PREFIX)/share/hachikuji/
	@echo "Add to ~/.zshrc, before zsh-syntax-highlighting:"
	@$(PREFIX)/bin/hachikuji init zsh

test: unit integration e2e startup memory

unit:
	$(CARGO) test --release

integration: all
	zsh tests/zsh.zsh

# Latency inside the helper (4a): 10,000 key presses replayed.
bench: all
	HACHIKUJI_INDEX=build/specs.idx $(BIN) bench bench/commands.txt

# Key press to updated popup, through a real zsh in a pty (4b).
e2e: all
	python3 tests/e2e_latency.py 600

startup: all
	python3 tests/startup.py 40

memory: all
	zsh tests/memory.zsh

# Release archives: binary + zsh file + index + licenses, for macOS (both archs) and Linux.
release: index
	tools/release.sh

clean:
	rm -rf target build
