# Run after sourcing the output of `hachikuji init nu`, with HACHIKUJI_INDEX set.
use std/assert

let work = mktemp -d
cd $work
touch README.md 'a&b.txt' '001.txt' 'my file.txt'
let complete = $env.config.completions.external.completer
for token in ['`READ`' '`READ' 'a&b' '001' '"my f"' '"my f' "'my f'" "r#'my f'#"] {
    let result = do $complete {command: [cat $token]}
    assert ($result != null) $'No completion for ($token)'
    assert (($result.completions | length) == 1) $'No unique file completion for ($token): ($result)'
    let expected = if ($token | str contains 'READ') { 'README.md ' } else if $token == 'a&b' { 'a&b.txt ' } else if $token == '001' { '001.txt ' } else { '"my file.txt" ' }
    assert equal $result.completions.0.value $expected
}
assert equal (do $complete {command: [git chec]}).completions.0.value 'checkout '
assert equal (do $complete {command: [git commit --mes]}).completions.0.value '--message '
assert equal (do $complete {command: [unknown-command '']}) null
mkdir 'my dir'
with-env {HOME: $work} {
    assert equal (do $complete {command: [cd '~/my']}).completions.0.value ($'($work)/my dir/' | to json --raw)
}
cd ..
rm -r $work
print 'Nushell completion checks passed'
