# hachikuji: build prompt

You are building `hachikuji`, an IDE-style completion popup for zsh, inspired by Fig and inshellisense, that costs almost no memory. Read this whole file before you write code. Work in milestones. Stop at the end of Milestone 0 and report.

## Why this exists

inshellisense uses about 95 MB per terminal (a Node launcher of about 31 MB and a binary of about 64 MB). With 12 terminals that is about 1.1 GB. It also wraps the shell in its own pseudo-terminal. Because of this, terminal apps that read the folder of the foreground process (Herdr, for example) see the wrapper, not the shell, and always show the start folder. Its theme also does not follow the system light and dark mode, so the dotfiles need a workaround.

`hachikuji` must fix all three problems: it must use very little memory, it must not wrap the shell, and its colors must follow the terminal theme with no extra code.

## Hard constraints

1. **No pty wrapper.** The user's zsh stays the terminal's foreground process. All drawing goes through ZLE (the zsh line editor). If a design needs a wrapper, it is wrong.
2. **No Node, no JS runtime, no Python at runtime.** The helper is one native binary. Use Rust (see "Language"). Build-time tools may use anything.
3. **Memory:** less than 4 MB of private (unshared) memory per shell for the helper. Measure with `vmmap --summary` or `footprint`, not only RSS. Data that all shells share (the spec index) is a read-only `mmap` file, so the OS shares its pages.
4. **Latency:** p99 below 16 ms on a recent Mac, for static specs, for two numbers: (a) inside the helper, from request read to response written, and (b) end to end, from key press to an updated popup on screen. Slow sources (generators that run commands) are async and never block typing.
5. **Startup:** adds less than 10 ms to zsh startup. The helper starts lazily, the first time the buffer is not empty, not in `.zshrc`.
6. **Fail silent.** If the helper crashes, hangs or is missing, the shell works as normal, with no popup and no error text. zsh must never block on the helper. Restart the helper at most once a minute.
7. **No network** at runtime. No telemetry. The one exception is `hachikuji update`, which the user runs; it calls the system `curl`, so the binary has no network code.
8. **zsh only** for v1. macOS first, Linux must build. Bash and fish are non-goals for now.
9. **Theme follows the terminal.** Use only the 16 ANSI palette colors (0 to 15), the default foreground and background, and attributes (bold, dim, reverse). Never use RGB or the 256-color cube (16 to 255): terminals do not change those colors when the theme changes. When the terminal switches between light and dark (for example Ghostty with `theme = light:Catppuccin Latte,dark:Catppuccin Mocha`), the popup follows with no code and no restart.

## Language

Rust. Reasons, from this spec:

- No garbage collector and no runtime. The 4 MB private memory budget is easy to keep. A Go process starts with a runtime, a GC heap and goroutine stacks, so it is close to 4 MB before it does any work.
- Mature crates for each part: `memmap2` (zero-copy index), `unicode-width` (cut rows to width), `toml` (config), `cargo-fuzz` (tokenizer fuzzing).
- One static binary for macOS (arm64 and x86_64) and Linux, with `cargo build --target`.
- Zig would also meet the budgets, but it is not at 1.0 yet. Each release breaks the language and the standard library, and it has fewer libraries for TOML, Unicode width and fuzzing.

## Architecture

```
zsh (ZLE widgets, ~300 lines of zsh)          hachikuji (Rust, one per shell)
  line-pre-redraw hook ── request ──────────▶  tokenizer → spec lookup → ranker → renderer
  zle -F fd handler   ◀── response ─────────  async generators (timeouts, cache)
  prints the lines it gets                     mmap: ~/.cache/hachikuji/specs.idx
```

### The zsh side (`hachikuji.zsh`)

- Trigger on `add-zle-hook-widget line-pre-redraw`, not by wrapping `self-insert`. This keeps it compatible with zsh-syntax-highlighting, zsh-autosuggestions and starship, which wrap widgets.
- `line-pre-redraw` runs on every redraw, also on the redraws that the response handler causes. Send a request only when `BUFFER` or `CURSOR` changed since the last request. If you do not do this, redraw → request → response → redraw loops forever.
- **Transport:** two FIFOs in a private folder (`mktemp -d`, mode 700). Open both on the zsh side with `exec {fd}<>` (read-write, so the open never blocks). Start the helper with `hachikuji serve <dir> &!` (disowned, so it has no job entry and is in its own process group). Do not use `coproc`: zsh has only one coproc slot, and users may need it. The helper deletes the folder after it opens the FIFOs.
- **Never block on a write.** Keep at most one request in flight. While one is in flight, keep only the newest pending request and send it when the response comes. Do not send a buffer larger than 8 KB (no popup for very long lines). If a request gets no response in 1 s, the helper is hung: kill it, close the fds, show no popup, and restart at most once a minute.
- Read responses with `zle -F <fd> <handler>`, so async results (generators) update the popup without blocking input.
- Send each request with a sequence number. Drop responses for an older sequence.
- `CURSOR` counts characters, but the protocol uses bytes. Convert it, for example with `${#${BUFFER[1,CURSOR]}}` under `local LC_ALL=C`.
- **Drawing:** the helper sends lines that are ready to print (borders, colors, padding, already cut to width). zsh only places them. Milestone 0 must choose between two methods:
  - (a) **`POSTDISPLAY` with newlines**, colored with `region_highlight`. ZLE then handles the bottom of the screen, multi-line prompts and resize. The risk: zsh-autosuggestions also writes `POSTDISPLAY`, so the popup must come after its ghost text, and both must survive each other's updates.
  - (b) **Escape codes** (save cursor, move, draw, restore). The risk: ZLE does not know these rows exist, and its redraw can clear them.

  Try (a) first. If neither is reliable, propose a fallback (`zle -M`, or zsh's own completion list) before you go on.
- Clear the popup on accept, on Esc, on `accept-line`, on `send-break` (Ctrl-C) and on `zle-line-finish`.
- Handle the bottom of the screen: if there are not enough rows below the cursor, the prompt must move up, as `fzf --height` does. Multi-line prompts (starship with two lines) must work. With method (a), ZLE does this.
- Handle resize (`SIGWINCH`) and redraw. Do not define `TRAPWINCH` if method (a) works without it: other plugins may need that function.
- **Keys.** Every key that hachikuji binds calls the widget that was bound before it when the popup is closed. Call the user's widget (`zle accept-line`), not the builtin (`zle .accept-line`), so that other plugins' wrappers still run. Source `hachikuji.zsh` before zsh-syntax-highlighting, which must load last.
  - Up/Down and Ctrl-N/Ctrl-P move the selection.
  - **Tab** accepts the selected item. The first item is selected when the popup opens.
  - **Enter** accepts the selected item only if the user moved the selection (Up, Down, Ctrl-N, Ctrl-P) since the popup opened. If not, Enter runs the line. This way Enter is a trigger when the user picks an item on purpose, and `git status⏎` still runs at once.
  - **After an accept,** insert the item. Add a space after a subcommand, an option or a complete argument. Add `/` after a folder. Then open the popup for the next token at once, so the user can keep pressing Tab (or move and press Enter) to build the line.
  - Esc closes the popup. zsh waits `KEYTIMEOUT` (default 0.4 s) after a bare Esc, because Esc also starts arrow-key sequences. Accept that delay. In vi mode, Esc with the popup closed must still enter command mode.
  - Right arrow at the end of the line keeps zsh-autosuggestions' behavior.

### The helper (`hachikuji`, Rust)

- **Process:** call `setsid()` at start, so the helper is never in the terminal's foreground process group. Then Ctrl-C does not reach it, and Herdr never picks it instead of zsh. Ignore SIGINT, SIGTSTP and SIGHUP. `chdir("/")`, and use the request's `PWD` only for generators. Exit on EOF on the request FIFO (the shell is gone) and on `EPIPE` on the response FIFO.
- **Protocol:** NUL-separated fields, one frame per request: protocol version, `seq`, `cursor` (byte offset), `PWD`, `COLUMNS`, `LINES`, `buffer`. The response gives `seq`, the insert text and kind for each row, and the rows as ready-to-print lines. Version the protocol.
- **Rendering:** the helper reads the config and renders each row: icon, name, description, border and the selection style. It cuts each line to `COLUMNS` with `unicode-width`. Count Nerd Font icons as width 1, and leave a 1-column space after each icon.
- **Tokenizer:** knows quotes, escapes, `|`, `&&`, `||`, `;`, `$( )`, backticks and subshells. It finds the command segment under the cursor and the token being typed. The shell sends aliases (`alias -L`) when the helper starts, and the helper expands them. Aliases defined after that are not seen until the next restart; that is acceptable for v1.
- **Sources, in order:**
  1. **Static specs** from the index. Support these Fig fields: subcommands, options, args, descriptions, `requiresArgument`, `isRepeatable`, `exclusiveOn`, `isPersistent` (options that subcommands inherit, such as `git -C`), `isVariadic`, `isOptional`, `requiresSeparator` (`--foo=bar`), `insertValue`, `hidden`, `priority`, and `isCommand` / `loadSpec` (for `sudo`, `npx`, `xargs`, `env`). Map `template: "filepaths"` and `template: "folders"` to the built-in generators.
  2. **Built-in generators**, written in Rust: files, folders, git branches, remotes and tags, and executables in `PATH`. Each one has a timeout (200 ms), runs in the request's `PWD`, and caches with a short TTL keyed on `PWD` (and the git `HEAD` mtime for git).
  3. **Script generators** from specs (`script` + `splitOn`) run through `/bin/sh -c` with the same timeout and cache. Skip generators that need JS (`postProcess` functions) unless they are ported to (2).
- **Ranking:** prefix match first, then fuzzy (subsequence) match. Tie-break by the spec's `priority`, then alphabetically. No ML, no usage history.
- **Memory discipline:** no global allocations that grow per request. Reuse buffers. Generator caches have a size cap (for example 1,000 entries). Fuzz the tokenizer.

### Spec compiler (build time only)

- Source: [withfig/autocomplete](https://github.com/withfig/autocomplete) (MIT). Pin one commit. Ship its license with the index. Specs are TypeScript.
- A build step (Bun or Node is fine here) loads each spec, drops functions, and writes the static tree plus the declarative generators as JSON.
- A Rust tool turns that JSON into one compact binary index (string table plus node table), read with `mmap` and zero-copy. Aim for less than 15 MB for all specs.
- Keep a list of which specs lost data (JS generators) so they can be ported later.
- Ship the index in the release. Users never run Node.

## Config

`~/.config/hachikuji/config.toml`. The helper reads it. Keep it small:

```toml
max_rows = 8
border = "rounded"        # rounded | square | none
nerd_font = true
descriptions = true
selected = "reverse"      # "reverse", or a palette index from 0 to 15
```

Default colors: borders and descriptions use palette 8 (bright black). Names use the default foreground. Kinds use palette colors: subcommand 4 (blue), option 5 (magenta), folder 6 (cyan), file and argument use the default foreground. The selected row uses reverse video. Check the defaults with Catppuccin Mocha and Latte: the selected row and the descriptions must be readable in both.

## Non-goals for v1

Bash, fish and nu; Windows; a GUI; AI suggestions; inline ghost text (zsh-autosuggestions does that); a plugin system; a shared daemon for all shells (look at this again only if the per-shell helper misses the memory budget); a history generator (zsh-autosuggestions covers history); ranking by usage frequency; a fallback to zsh's own completions for commands with no spec (capture through `compadd`, as fzf-tab does; consider it after v1).

## Milestones

**0. Rendering spike (stop and report after this).** Pure zsh, no helper: a fake list of 8 fixed items in a popup, drawn from `line-pre-redraw`. Try drawing method (a), then (b). Prove these: the bottom-of-screen scroll, a two-line starship prompt, resize, zsh-syntax-highlighting and zsh-autosuggestions still work, Tab and Enter follow the key rules, a paste of 1,000 characters is not slow, and the popup is readable in Catppuccin Mocha and Latte and follows a light/dark switch. Report what broke and which method you chose. If ZLE drawing can't be made reliable, propose a fallback before you go on.

**1. Helper with git only.** The tokenizer, the protocol, the FIFO transport, a hand-written git spec, file and folder completion, and the zsh side wired to the helper. Prove that Herdr and Ghostty show the right folder after `cd` while the helper runs, and that Ctrl-C at the prompt does not kill the helper. Kill the helper with `SIGSTOP` and check that the prompt does not hang. Measure latency and memory here, not at the end.

**2. Spec compiler.** All the Fig specs, static parts, in the `mmap` index.

**3. Async generators.** `zle -F` updates, the built-in generators, script generators, caches.

**4. Packaging.** A Homebrew tap formula, `hachikuji init zsh` prints the line to source, the config file and the README.

## Tests

- Unit tests for the tokenizer: quotes, pipes, subshells, aliases, the cursor in the middle of a line, multibyte text.
- Golden tests for the spec compiler: a few specs in, index lookups out.
- A latency bench: replay 10,000 recorded key presses through the helper and report p50, p99 and the maximum (number 4a).
- A memory check script: start 12 shells, type in each, report the private memory per helper.
- A zsh integration test that drives a real zsh in a pty (`zsh/zpty` or `expect`), types, and checks the screen. It also measures the end-to-end latency (number 4b).

## Done means

- All the hard constraints, with numbers measured on this Mac and written in the README.
- Daily use for a week with no hung prompt and no corrupt screen.
- inshellisense removed from the dotfiles. Delete the `.zshrc` workaround for Herdr and the theme workaround, because neither is needed.
