# Shell for the nushell demo GIF (docs/demo/nu.tape): no personal history or settings.
$env.config.show_banner = false
$env.PROMPT_COMMAND = {|| $"(ansi blue)($env.PWD | path basename)(ansi reset) " }
$env.PROMPT_INDICATOR = $"(ansi magenta)❯(ansi reset) "
$env.PROMPT_COMMAND_RIGHT = ""
$env.HACHIKUJI_INDEX = ($env.PWD | path join build/specs.idx)  # `make` builds it
# The popup next to the line (README, Nushell).
$env.config.menus ++= [{name: completion_menu, only_buffer_difference: false, marker: $"(ansi magenta)❯(ansi reset) ", type: {layout: ide}, style: {}}]
