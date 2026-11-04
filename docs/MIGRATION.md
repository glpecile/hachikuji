# Migrate to hachikuji

This guide replaces Kiro CLI, Amazon Q Developer CLI, Fig or inshellisense with hachikuji. It is written for coding agents that do the work for a user, and people can follow it too. Run the steps in order. Each step has a check.

hachikuji supports zsh only, on macOS and Linux (aarch64). For bash or fish, stop and tell the user.

## Rules for agents

1. **Back up each shell startup file before you edit it.** Run `cp ~/.zshrc ~/.zshrc.pre-hachikuji` (also for each other file you change).
2. **Remove only the lines this guide names.** Other plugins, prompts and aliases stay.
3. **Show the user what you will delete** (the lines, the folders) before you delete it. Data folders can hold the user's own settings.
4. **Do not kill shells.** inshellisense, Kiro and Amazon Q wrap the shell or attach to it. Tell the user to close their old terminal tabs at the end. Do not kill them yourself, because you may be running inside one.
5. **If `~/.zshrc` is a symlink** (dotfiles repo), edit the target file and tell the user that the change is in that repo.

## 1. Find what is installed

```sh
for c in kiro-cli q fig inshellisense; do printf '%-14s %s\n' $c "$(whence -p $c 2>/dev/null || echo -)"; done
ls -d /Applications/{Fig,"Amazon Q","Kiro CLI"}.app ~/.fig ~/.kiro ~/.inshellisense ~/.inshellisenserc \
  ~/.config/inshellisense ~/.local/share/{inshellisense,amazon-q,kiro-cli} \
  "$HOME/Library/Application Support/"{amazon-q,kiro-cli} 2>/dev/null
brew list --cask 2>/dev/null | grep -xE 'fig|amazon-q|kiro-cli'
brew list --formula 2>/dev/null | grep -x inshellisense
npm ls -g --depth=0 2>/dev/null | grep -i inshellisense
# Lines in the startup files (zsh, bash, and fish for completeness):
grep -nEi '(pre|post) block|amazon-q|kiro-cli|\.fig/|inshellisense|ISTERM' \
  ~/.zshrc ~/.zprofile ~/.zshenv ~/.bashrc ~/.bash_profile ~/.bash_login ~/.profile \
  ~/.config/fish/conf.d/{00_fig_pre,99_fig_post}.fish 2>/dev/null
true  # grep and ls exit non-zero when nothing is found: that is not an error
```

Go to the section of each tool that the output shows. Skip the others.

## 2. Remove the old tool

### Kiro CLI (3.x)

Kiro CLI is the new name of Amazon Q Developer CLI. Version 3 is a chat agent only: it has no completion popup and adds no lines to the shell startup files. The Homebrew casks `kiro-cli` and `amazon-q` both install it now.

1. Uninstall. Without a terminal, `kiro-cli uninstall` stops and asks for `-y`, so pass it:
   ```sh
   kiro-cli uninstall -y        # removes ~/.local/bin/kiro-cli and logs out
   brew uninstall --cask kiro-cli   # instead, if brew lists kiro-cli (or amazon-q)
   ```
2. If `/Applications/Kiro CLI.app` exists (the macOS install script puts it there), quit it and move it to the Trash.
3. Remove what is left, after you show it to the user. The uninstaller keeps these on purpose:
   - `~/.local/share/kiro-cli`, `~/Library/Application Support/kiro-cli`, `~/Library/Caches/dev.kiro.cli`, `~/Library/Preferences/dev.kiro.cli.plist`, `~/Library/Caches/com.amazon.codewhisperer`, `~/Library/LaunchAgents/com.amazon.codewhisperer.launcher.plist`, `~/Library/WebKit/com.amazon.codewhisperer`.
   - `~/.kiro` holds the user's Kiro settings, agents and sessions, and the Kiro IDE uses it too. Delete it only if the user says so.
4. Earlier versions with a desktop app (Amazon Q, Kiro CLI 1 and 2) wrote shell lines. If the grep from step 1 still shows a "pre block" or "post block", go to the Amazon Q section, step 2.

### Amazon Q Developer CLI (the `q` command)

1. Uninstall: `q uninstall`. Installed with Homebrew: `brew uninstall --cask amazon-q`. Ubuntu package: `sudo apt-get remove amazon-q`, then `sudo apt-get purge amazon-q`.
2. Run the grep from step 1 again. The app wrote a pair of lines at the top and at the bottom of `.zshrc` and `.zprofile`, and of `.bashrc` and `.bash_profile`, `.bash_login` or `.profile`:
   ```sh
   # Amazon Q pre block. Keep at the top of this file.
   [[ -f "${HOME}/.local/share/amazon-q/shell/zshrc.pre.zsh" ]] && builtin source "${HOME}/.local/share/amazon-q/shell/zshrc.pre.zsh"
   ```
   The comment says `post block` and "bottom" for the second pair. Older versions write `CodeWhisperer` instead of `Amazon Q`, and newer ones may write another product name. Delete each comment line that ends in "pre block. Keep at the top of this file." or "post block. Keep at the bottom of this file.", and the `source` line after it. For fish, delete `~/.config/fish/conf.d/00_fig_pre.fish` and `99_fig_post.fish`.
3. Remove what is left, after you show it to the user: `~/.local/share/amazon-q`, `~/Library/Application Support/amazon-q`, `/Applications/Amazon Q.app`.

### Fig

The Fig service shut down on 1 September 2024, and Homebrew no longer has a `fig` cask. An old install can still edit the shell.

1. If `fig --help` lists `uninstall`, run `fig uninstall`. If `brew list --cask` shows `fig`, run `brew uninstall --cask --force fig`.
2. Run the grep from step 1 again. Delete these lines if they are there: `[ -s ~/.fig/shell/pre.sh ] && source ~/.fig/shell/pre.sh`, `[ -s ~/.fig/fig.sh ] && source ~/.fig/fig.sh`, any other line that sources a file from `~/.fig/`, and the "pre block" or "post block" comment line above each one.
3. Remove what is left, after you show it to the user: `~/.fig`, `/Applications/Fig.app`.

### inshellisense

1. Remove its generated files and settings, then the package:
   ```sh
   inshellisense uninstall                      # removes its cache and configuration
   brew uninstall inshellisense                 # if brew lists it
   npm uninstall -g @microsoft/inshellisense    # if npm lists it
   ```
2. Delete its lines from `~/.zshrc`: the line that `is init zsh` printed (it sources `~/.local/share/inshellisense/init/zsh/init.zsh`, or a file under `~/.inshellisense`), and lines the user added for it, for example an `is()` wrapper function or an `[[ -n $ISTERM ]]` check.
3. Remove what is left, after you show it to the user: `~/.local/share/inshellisense`, `~/.inshellisense`, `~/.config/inshellisense`, `~/.inshellisenserc`.

**Check for step 2:** the grep from step 1 prints nothing, and `whence -p kiro-cli q fig inshellisense` prints nothing.

## 3. Install hachikuji

Use the first method that works. If the script URL returns 404, no release is published yet. If `git clone` asks for a login, the repository is not public yet. In both cases, stop and tell the user.

```sh
# Script: macOS (arm64, x86_64) and Linux aarch64. Installs to ~/.local, or to $PREFIX.
curl -fsSL https://github.com/glpecile/hachikuji/releases/latest/download/install.sh | sh

# Homebrew:
brew install glpecile/tap/hachikuji

# From source (needs rustup, bun and git):
git clone https://github.com/glpecile/hachikuji && cd hachikuji && make install
```

Each method prints a line that starts with `HACHIKUJI_BIN=`. To print it again: `hachikuji init zsh` (with Homebrew), or `~/.local/bin/hachikuji init zsh`.

**Check:** `hachikuji complete 'git chec'` (or `~/.local/bin/hachikuji complete 'git chec'`) prints a first line that starts with `count=`.

## 4. Load it in zsh

Add the printed line to `~/.zshrc`:

- **Before** the line that sources zsh-syntax-highlighting, which must stay last.
- **After** zsh-autosuggestions, oh-my-zsh and the prompt (starship, powerlevel10k). The order among these does not matter.
- Add the printed line itself, not `eval "$(hachikuji init zsh)"`. The eval starts a process at each shell start.

```zsh
# hachikuji: IDE-style completion popup (https://github.com/glpecile/hachikuji)
HACHIKUJI_BIN='/Users/you/.local/bin/hachikuji' source '/Users/you/.local/share/hachikuji/hachikuji.zsh'
source /opt/homebrew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh
```

Optional settings go in `~/.config/hachikuji/config.toml` (see the README). For a user who did not like a popup that opens on each key, set `open = "tab"`.

## 5. Verify

```sh
zsh -i -c 'typeset -f _hk_pre_redraw >/dev/null && echo hachikuji-loaded'
```

The output must be `hachikuji-loaded`, and the grep from step 1 must still print nothing. (Do not check `$ISTERM` or similar variables: your own process may run inside an old wrapped tab and pass them on.) Then tell the user:

1. Close the old terminal tabs and open new ones. Old tabs keep the old tool until they close; `exec zsh` does not leave a wrapper such as inshellisense.
2. Type `git ch`. A popup opens under the line. Up and Down select, Tab inserts, Esc closes. When no popup is open, two Tabs run zsh's own completion.

## Undo

```sh
cp ~/.zshrc.pre-hachikuji ~/.zshrc    # and each other file you backed up
rm -rf ~/.local/bin/hachikuji ~/.local/share/hachikuji ~/.cache/hachikuji   # or: brew uninstall hachikuji
```

Then install the old tool again with its own installer.
