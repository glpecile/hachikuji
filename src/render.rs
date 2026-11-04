//! Renders candidates as ready-to-print popup lines plus region_highlight entries.
//! Colors are palette indices 0-15, the default colors and attributes only, so the popup
//! follows the terminal theme. Offsets are in characters, as zsh counts them.

use crate::complete::{Cand, Kind};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Border {
    Rounded,
    Square,
    None,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub max_rows: usize,
    pub border: Border,
    pub nerd_font: bool,
    pub descriptions: bool,
    /// None: reverse video. Some(n): background palette color n.
    pub selected: Option<u8>,
}

impl Default for Config {
    fn default() -> Config {
        Config { max_rows: 8, border: Border::Rounded, nerd_font: true, descriptions: true, selected: None }
    }
}

impl Config {
    /// Reads `~/.config/hachikuji/config.toml`. Bad or missing values keep their default.
    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        let Ok(t) = text.parse::<toml::Table>() else { return c };
        if let Some(n) = t.get("max_rows").and_then(|v| v.as_integer()) {
            c.max_rows = n.clamp(1, 50) as usize;
        }
        match t.get("border").and_then(|v| v.as_str()) {
            Some("square") => c.border = Border::Square,
            Some("none") => c.border = Border::None,
            _ => {}
        }
        if let Some(b) = t.get("nerd_font").and_then(|v| v.as_bool()) {
            c.nerd_font = b;
        }
        if let Some(b) = t.get("descriptions").and_then(|v| v.as_bool()) {
            c.descriptions = b;
        }
        if let Some(n) = t.get("selected").and_then(|v| v.as_integer()) {
            if (0..=15).contains(&n) {
                c.selected = Some(n as u8);
            }
        }
        c
    }

    pub fn load() -> Config {
        let home = std::env::var("HOME").unwrap_or_default();
        let dir = std::env::var("XDG_CONFIG_HOME").unwrap_or(format!("{home}/.config"));
        Config::parse(&std::fs::read_to_string(format!("{dir}/hachikuji/config.toml")).unwrap_or_default())
    }
}

pub struct Popup {
    /// Starts with "\n": it goes after the buffer in POSTDISPLAY.
    pub text: String,
    /// "start end spec" with offsets relative to the start of `text`.
    pub hl: Vec<String>,
    pub sel: usize,
}

fn kind_style(k: Kind) -> (&'static str, &'static str) {
    // (Nerd Font icon, color). Descriptions and names use the default foreground: palette 8
    // is not readable as text in Catppuccin Mocha (2.5:1) or Latte (1.9:1).
    match k {
        Kind::Cmd => ("\u{f120}", "fg=2"),
        Kind::Sub => ("\u{f0a9}", "fg=4"),
        Kind::Opt => ("\u{f1de}", "fg=5"),
        Kind::Folder => ("\u{f07b}", "fg=6"),
        Kind::File => ("\u{f15b}", "fg=default"),
        Kind::Arg => ("\u{f0da}", "fg=default"),
    }
}

/// Display width of what `put` writes.
fn width(s: &str) -> usize {
    s.chars().filter(|&c| safe(c)).map(|c| c.width().unwrap_or(0)).sum()
}

/// Characters whose width zsh (libc wcwidth) and the terminal agree on. Emoji, symbols and
/// zero-width characters (variation selectors, ZWJ, combining marks) are left out: one
/// disagreement shifts ZLE's idea of the cursor, and every later redraw leaves old text
/// behind ("⛰️" in a Fig description did this). Box drawing, "…" and CJK are fine.
fn safe(c: char) -> bool {
    let u = c as u32;
    let w = c.width().unwrap_or(0);
    w > 0
        && !(0x2190..=0x24ff).contains(&u) // arrows, technical, enclosed alphanumerics
        && !(0x25a0..=0x2bff).contains(&u) // shapes, misc symbols, dingbats, arrows
        && !(0xe000..=0xf8ff).contains(&u) // private use (only our own icons may be there)
        && !(0x1f000..=0x1faff).contains(&u) // emoji and pictographs
        && !(0xfe00..=0xfe0f).contains(&u)
}

/// Appends `s` cut to `w` columns and padded with spaces. Returns the characters written.
fn put(out: &mut String, s: &str, w: usize) -> usize {
    let (mut used, mut n) = (0, 0);
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if !safe(c) {
            continue;
        }
        if used + cw > w {
            break;
        }
        out.push(c);
        used += cw;
        n += 1;
    }
    for _ in used..w {
        out.push(' ');
        n += 1;
    }
    n
}

pub fn render(c: &[Cand], sel: usize, cols: usize, lines: usize, cfg: &Config) -> Option<Popup> {
    if c.is_empty() {
        return None;
    }
    let sel = sel % c.len();
    let b = cfg.border != Border::None;
    let edge = if b { 2 } else { 0 };
    // Rows that fit: leave room for a two-line prompt, the buffer line and the borders.
    let rows = cfg.max_rows.min(c.len()).min(lines.saturating_sub(3 + edge));
    let maxw = cols.saturating_sub(1);
    if rows == 0 || maxw < edge + 8 {
        return None;
    }
    let first = if sel >= rows { sel + 1 - rows } else { 0 };
    let shown = &c[first..first + rows];
    let icon_w = if cfg.nerd_font { 2 } else { 0 };
    let name_w = shown.iter().map(|x| width(&x.name)).max().unwrap_or(0).min(40);
    let desc_w = if cfg.descriptions { shown.iter().map(|x| width(&x.desc)).max().unwrap_or(0).min(51) } else { 0 };
    let inner_full = 1 + icon_w + name_w + if desc_w > 0 { 2 + desc_w } else { 0 } + 1;
    // Show the position when the list scrolls: ╰─ 3/120 ──╯
    let pos = if c.len() > rows && b { format!(" {}/{} ", sel + 1, c.len()) } else { String::new() };
    let inner = inner_full.max(pos.len() + 2).min(maxw - edge);
    let name_w = name_w.min(inner.saturating_sub(2 + icon_w));
    let (tl, tr, bl, br, h, v) = match cfg.border {
        Border::Square => ('┌', '┐', '└', '┘', '─', '│'),
        _ => ('╭', '╮', '╰', '╯', '─', '│'),
    };
    let sel_style = match cfg.selected {
        None => "fg=default,standout".to_string(),
        Some(n) => format!("fg=default,bg={n}"),
    };
    let mut t = String::with_capacity(rows * (inner + 4) * 3);
    let mut hl = Vec::with_capacity(rows * 4 + 2);
    let mut at = 0usize; // characters written so far
    let line = |t: &mut String, at: &mut usize| {
        t.push('\n');
        *at += 1;
    };
    if b {
        line(&mut t, &mut at);
        let start = at;
        t.push(tl);
        (0..inner).for_each(|_| t.push(h));
        t.push(tr);
        at += inner + 2;
        hl.push(format!("{start} {at} fg=8"));
    }
    for (i, x) in shown.iter().enumerate() {
        line(&mut t, &mut at);
        let row = at;
        if b {
            t.push(v);
            at += 1;
            hl.push(format!("{row} {at} fg=8"));
        }
        let (icon, color) = kind_style(x.kind);
        let in_start = at;
        let mut used = 0;
        at += put(&mut t, " ", 1);
        used += 1;
        if cfg.nerd_font && inner >= used + 2 {
            let s = at;
            t.push_str(icon); // a Nerd Font icon: width 1, as the plan says
            at += 1;
            at += put(&mut t, " ", 1);
            used += 2;
            hl.push(format!("{s} {} {color}", s + 1));
        }
        let name_start = at;
        at += put(&mut t, &x.name, name_w.min(inner - used));
        used += name_w.min(inner - used);
        if !cfg.nerd_font {
            hl.push(format!("{name_start} {at} {color}"));
        }
        if desc_w > 0 && inner > used + 2 {
            at += put(&mut t, "  ", 2);
            used += 2;
            at += put(&mut t, &x.desc, inner - used);
            used = inner;
        }
        at += put(&mut t, "", inner - used);
        if first + i == sel {
            hl.push(format!("{in_start} {at} {sel_style}"));
        }
        if b {
            let s = at;
            t.push(v);
            at += 1;
            hl.push(format!("{s} {at} fg=8"));
        }
    }
    if b {
        line(&mut t, &mut at);
        let start = at;
        t.push(bl);
        let mut n = 0;
        if inner > pos.len() + 1 && !pos.is_empty() {
            t.push(h);
            t.push_str(&pos);
            n = 1 + pos.len();
        }
        (n..inner).for_each(|_| t.push(h));
        t.push(br);
        at += inner + 2;
        hl.push(format!("{start} {at} fg=8"));
    }
    Some(Popup { text: t, hl, sel })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Columns on screen: like `width`, but our Nerd Font icons count (1 column each).
    fn cols(s: &str) -> usize {
        s.chars().map(|c| c.width().unwrap_or(0)).sum()
    }

    fn cand(name: &str, desc: &str, kind: Kind) -> Cand {
        Cand { name: name.into(), desc: desc.into(), kind, priority: 50, score: 0, body: name.into(), suffix: " ", raw: false }
    }

    #[test]
    fn rows_fit_the_width_and_offsets_match() {
        let c: Vec<Cand> = (0..20).map(|i| cand(&format!("name{i}"), "a long description with wide 漢字 text", Kind::Sub)).collect();
        for cols in [20, 40, 80, 200] {
            let p = render(&c, 3, cols, 24, &Config::default()).unwrap();
            let lines: Vec<&str> = p.text.split('\n').skip(1).collect();
            assert_eq!(lines.len(), 8 + 2);
            for l in &lines {
                assert!(super::tests::cols(l) < cols, "{cols}: {l}");
                assert_eq!(super::tests::cols(l), super::tests::cols(lines[0]), "all lines have the same width");
            }
            let chars = p.text.chars().count();
            for h in &p.hl {
                let mut it = h.split(' ');
                let (s, e): (usize, usize) = (it.next().unwrap().parse().unwrap(), it.next().unwrap().parse().unwrap());
                assert!(s < e && e <= chars, "{h} {chars}");
            }
        }
    }

    #[test]
    fn scrolls_to_the_selection_and_shows_position() {
        let c: Vec<Cand> = (0..20).map(|i| cand(&format!("n{i:02}"), "", Kind::Opt)).collect();
        let p = render(&c, 15, 80, 24, &Config::default()).unwrap();
        assert!(p.text.contains("n15") && !p.text.contains("n07"));
        assert!(p.text.contains(" 16/20 "));
        let p = render(&c, 25, 80, 24, &Config::default()).unwrap();
        assert_eq!(p.sel, 5);
    }

    #[test]
    fn only_palette_colors() {
        let c = vec![cand("a", "b", Kind::Sub), cand("c", "", Kind::Folder)];
        for cfg in [Config::default(), Config::parse("selected = 12\nnerd_font = false\nborder = \"none\"")] {
            let p = render(&c, 0, 80, 24, &cfg).unwrap();
            for h in &p.hl {
                for part in h.split(' ').nth(2).unwrap().split(',') {
                    if let Some(n) = part.strip_prefix("fg=").or(part.strip_prefix("bg=")) {
                        assert!(n == "default" || n.parse::<u8>().is_ok_and(|n| n < 16), "{h}");
                    }
                }
            }
        }
    }

    #[test]
    fn emoji_and_zero_width_characters_are_dropped() {
        let c = vec![cand("git-cliff", "changelog generator ⛰️ ok 😀 → 漢字 a\u{0301}", Kind::Cmd)];
        let p = render(&c, 0, 80, 24, &Config::default()).unwrap();
        let row = p.text.split('\n').nth(2).unwrap();
        assert!(row.contains("changelog generator  ok   漢字 a"), "{row}");
        assert!(!row.contains('⛰') && !row.contains('\u{fe0f}') && !row.contains('😀'));
        assert_eq!(cols(row), cols(p.text.split('\n').nth(1).unwrap()));
    }

    #[test]
    fn small_screens_and_config() {
        let c = vec![cand("a", "b", Kind::Sub)];
        assert!(render(&c, 0, 80, 4, &Config::default()).is_none());
        assert!(render(&c, 0, 5, 24, &Config::default()).is_none());
        let cfg = Config::parse("max_rows = 3\nborder = \"square\"\ndescriptions = false\nselected = 99");
        assert_eq!((cfg.max_rows, cfg.border, cfg.descriptions, cfg.selected), (3, Border::Square, false, None));
        assert!(!render(&c, 0, 80, 24, &cfg).unwrap().text.contains('b'));
    }
}
