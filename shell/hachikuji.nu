# hachikuji for nushell. Nushell's completion menu shows the items; hachikuji supplies them
# for external commands (git, docker, make, ...). Nushell completes its own commands itself.
# When hachikuji has nothing (no spec for the command), nushell completes file names.
let hachikuji_bin = r#'@BIN@'#
$env.config.completions.external.completer = {|place|
    ^$hachikuji_bin complete --nu ($place.command | str join ' ') | from json
}
