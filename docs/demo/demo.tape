# Records docs/demo.gif. From the repo root, after `make`: vhs docs/demo/demo.tape
# Needs vhs, and the GeistMono Nerd Font for the icons.
Output docs/demo.gif
Require zsh
Set Shell zsh
Set FontFamily "GeistMono Nerd Font"
Set FontSize 20
Set Width 1100
Set Height 540
Set Padding 24
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms

Hide
Type "source docs/demo/zshrc" Enter
Sleep 1s
Ctrl+L
Sleep 500ms
Show

# Subcommands: move with Up and Down, Tab inserts, and the next popup opens at once.
Type "git ch" Sleep 1.2s
Down Sleep 700ms Up Sleep 700ms
Tab Sleep 1.5s
Type "ma" Sleep 900ms
Tab Sleep 1.3s
Ctrl+U Sleep 500ms

# Folders only for cd. A folder insert opens the popup for its contents.
Type "cd " Sleep 1.2s
Down Sleep 500ms Down Sleep 500ms
Tab Sleep 1.6s
Ctrl+U Sleep 500ms

# Targets from the Makefile.
Type "make " Sleep 1.6s
Ctrl+U Sleep 500ms

# Options with descriptions.
Type "docker run --r" Sleep 1.4s
Tab Sleep 1.6s
Ctrl+U Sleep 800ms
