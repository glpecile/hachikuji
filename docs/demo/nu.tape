# Records docs/demo-nu.gif. From the repo root, after `make`: vhs docs/demo/nu.tape
# Needs vhs and nu.
Output docs/demo-nu.gif
Require nu
Set Shell nu
Set FontFamily "GeistMono Nerd Font"
Set FontSize 20
Set Width 1100
Set Height 540
Set Padding 24
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms

Hide
Type "source docs/demo/config.nu" Enter
# `source` needs a path known before the line runs, so the file is saved first.
Type "target/release/hachikuji init nu | save -f /tmp/hachikuji-demo.nu" Enter
Type "source /tmp/hachikuji-demo.nu" Enter
Sleep 1s
Ctrl+L
Sleep 500ms
Show

# Subcommands: Tab opens the menu, Down and Up move, Enter inserts.
Type "git ch" Sleep 500ms
Tab Sleep 1.2s
Down Sleep 700ms Up Sleep 700ms
Enter Sleep 1s
Type "ma" Sleep 500ms
Tab Sleep 1.2s
Enter Sleep 1.3s
Ctrl+U Sleep 500ms

# Targets from the Makefile.
Type "make " Sleep 500ms
Tab Sleep 1.6s
Escape Ctrl+U Sleep 500ms

# Options with descriptions.
Type "docker run --r" Sleep 500ms
Tab Sleep 1.2s
Down Sleep 700ms Down Sleep 700ms
Enter Sleep 1.6s
Ctrl+U Sleep 800ms
