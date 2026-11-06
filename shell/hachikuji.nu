# hachikuji for nushell. Nushell's completion menu shows the items; hachikuji supplies them
# for external commands (git, docker, make, ...). Nushell completes its own commands itself.
# When hachikuji has nothing (no spec for the command), nushell completes file names.
let hachikuji_bin = r#'@BIN@'#
$env.config.completions.external.completer = {|place|
    let words = $place.command | each {|word|
        if ($word | str starts-with '`') {
            $word | str trim --char '`'
        } else if ($word | str starts-with "'") or ($word | str starts-with '"') {
            let quote = $word | str substring 0..0
            let closed = if ($word | str length) > 1 and ($word | str ends-with $quote) { $word } else { $word + $quote }
            try { $closed | from nuon } catch { $word }
        } else if ($word | str starts-with "r'") or ($word | str starts-with 'r#') {
            try { $word | from nuon | into string } catch { $word }
        } else {
            $word
        }
    }
    ^$hachikuji_bin complete --nu ($words | to json --raw) | from json
}
