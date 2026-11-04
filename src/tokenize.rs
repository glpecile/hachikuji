//! Finds the command segment under the cursor and the word being typed.
//! Knows quotes, escapes, `| || && ; &`, `$( )`, backticks, subshells and redirections.
//! Only the text before the cursor matters for completion.

#[derive(Debug, Default, PartialEq)]
pub struct Line {
    /// Complete words of the segment, quotes and escapes removed. Redirections are left out.
    pub words: Vec<String>,
    /// The word being typed, quotes and escapes removed.
    pub cur: String,
    /// Byte offset in the buffer where the word being typed starts.
    pub cur_start: usize,
    /// The quote that is open at the cursor, if any.
    pub quote: Option<char>,
    /// The word being typed is the target of a redirection (`> file`).
    pub redirect: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Q {
    None,
    Single,
    Double,
    Dollar, // $'...'
}

struct Seg {
    words: Vec<String>,
    word: Option<(usize, String)>,
    quote: Q,
    /// What closes this segment: b')' or b'`'; 0 for the top level.
    close: u8,
    /// The next word is a redirection target.
    redir_next: bool,
    redir_cur: bool,
    /// The quote state of the outer segment, restored on close.
    outer_quote: Q,
}

impl Seg {
    fn new(close: u8, outer_quote: Q) -> Seg {
        Seg { words: vec![], word: None, quote: Q::None, close, redir_next: false, redir_cur: false, outer_quote }
    }
    fn start(&mut self, at: usize) -> &mut String {
        if self.word.is_none() {
            self.redir_cur = std::mem::take(&mut self.redir_next);
        }
        &mut self.word.get_or_insert_with(|| (at, String::new())).1
    }
    fn end_word(&mut self) {
        if let Some((_, w)) = self.word.take() {
            if !self.redir_cur {
                self.words.push(w);
            }
            self.redir_cur = false;
        }
    }
    fn new_command(&mut self) {
        self.end_word();
        self.words.clear();
        self.redir_next = false;
    }
}

pub fn parse(buf: &str, cursor: usize) -> Line {
    let mut cursor = cursor.min(buf.len());
    while !buf.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let s = &buf[..cursor];
    let b = s.as_bytes();
    let mut st: Vec<Seg> = vec![Seg::new(0, Q::None)];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let ch = s[i..].chars().next().unwrap();
        let n = ch.len_utf8();
        let next = b.get(i + 1).copied();
        let depth = st.len();
        let top = st.last_mut().unwrap();
        match top.quote {
            Q::Single => {
                if c == b'\'' {
                    top.quote = Q::None;
                } else {
                    top.start(i).push(ch);
                }
            }
            Q::Dollar => match c {
                b'\'' => top.quote = Q::None,
                b'\\' if next.is_some() => {
                    let nc = s[i + 1..].chars().next().unwrap();
                    let e = match nc {
                        'n' => '\n',
                        't' => '\t',
                        'e' | 'E' => '\x1b',
                        x => x,
                    };
                    top.start(i).push(e);
                    i += nc.len_utf8();
                }
                _ => top.start(i).push(ch),
            },
            Q::Double => match c {
                b'"' => top.quote = Q::None,
                b'\\' if matches!(next, Some(b'$' | b'`' | b'"' | b'\\' | b'\n')) => {
                    if next != Some(b'\n') {
                        top.start(i).push(next.unwrap() as char);
                    }
                    i += 1;
                }
                b'$' if next == Some(b'(') => {
                    top.start(i);
                    st.push(Seg::new(b')', Q::Double));
                    i += 1;
                }
                b'`' => {
                    top.start(i);
                    st.push(Seg::new(b'`', Q::Double));
                }
                _ => top.start(i).push(ch),
            },
            Q::None => match c {
                b' ' | b'\t' => top.end_word(),
                b'\n' | b';' => top.new_command(),
                b'&' => {
                    if top.redir_cur || matches!(top.word.as_ref().map(|w| w.1.as_str()), Some(">" | "<")) {
                        top.start(i).push('&'); // >&2 is a redirection, not a separator
                    } else {
                        top.new_command();
                        if matches!(next, Some(b'&' | b'|' | b'>')) {
                            i += 1;
                        }
                    }
                }
                b'|' => {
                    top.new_command();
                    if matches!(next, Some(b'|' | b'&')) {
                        i += 1;
                    }
                }
                b'>' | b'<' => {
                    // 2>file: the digits before > are a file descriptor, not a word.
                    if top.word.as_ref().is_some_and(|w| w.1.bytes().all(|d| d.is_ascii_digit())) {
                        top.word = None;
                        top.redir_cur = false;
                    }
                    top.end_word();
                    while matches!(b.get(i + 1), Some(b'>' | b'<' | b'|' | b'!')) {
                        i += 1;
                    }
                    if b.get(i + 1) == Some(&b'&') {
                        i += 1; // >&: the target is an fd or a file
                    }
                    top.redir_next = true;
                }
                b'\\' => {
                    if let Some(nc) = s[i + 1..].chars().next() {
                        if nc != '\n' {
                            top.start(i).push(nc);
                        }
                        i += nc.len_utf8();
                    } else {
                        top.start(i); // a trailing backslash starts a word
                    }
                }
                b'\'' => {
                    top.start(i);
                    top.quote = Q::Single;
                }
                b'"' => {
                    top.start(i);
                    top.quote = Q::Double;
                }
                b'$' if next == Some(b'\'') => {
                    top.start(i);
                    top.quote = Q::Dollar;
                    i += 1;
                }
                b'$' if next == Some(b'(') => {
                    top.start(i);
                    st.push(Seg::new(b')', Q::None));
                    i += 1;
                }
                b'(' if top.word.is_none() && top.words.is_empty() => st.push(Seg::new(b')', Q::None)),
                b')' if top.close == b')' && depth > 1 => {
                    let q = top.outer_quote;
                    st.pop();
                    st.last_mut().unwrap().quote = q;
                }
                b'`' if top.close == b'`' && depth > 1 => {
                    let q = top.outer_quote;
                    st.pop();
                    st.last_mut().unwrap().quote = q;
                }
                b'`' => {
                    top.start(i);
                    st.push(Seg::new(b'`', Q::None));
                }
                b'#' if top.word.is_none() => {
                    // A comment runs to the end of the line.
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                _ => top.start(i).push(ch),
            },
        }
        i += n;
    }
    let mut top = st.pop().unwrap();
    let quote = match top.quote {
        Q::None => None,
        Q::Single | Q::Dollar => Some('\''),
        Q::Double => Some('"'),
    };
    let redirect = if top.word.is_some() { top.redir_cur } else { top.redir_next };
    let (cur_start, cur) = top.word.take().unwrap_or((cursor, String::new()));
    Line { words: top.words, cur, cur_start, quote, redirect }
}

/// Expands an alias in the first word, as zsh does (up to 5 levels, no self loops).
pub fn expand_aliases(line: &mut Line, aliases: &[(String, String)]) {
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..5 {
        let Some(first) = line.words.first() else { return };
        if seen.contains(first) {
            return;
        }
        let Some((_, val)) = aliases.iter().find(|(k, _)| k == first) else { return };
        seen.push(first.clone());
        let v = parse(val, val.len());
        let mut words = v.words;
        if !v.cur.is_empty() {
            words.push(v.cur);
        }
        line.words.splice(0..1, words);
    }
}

/// Parses `alias -L` output: lines like `alias gs='git status'`. Global and suffix aliases are skipped.
pub fn parse_aliases(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in text.lines() {
        let p = parse(l, l.len());
        let mut w = p.words;
        w.push(p.cur);
        if w.len() == 2 && w[0] == "alias" {
            if let Some((k, v)) = w[1].split_once('=') {
                out.push((k.to_string(), v.to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Line {
        parse(s, s.len())
    }
    fn w(l: &Line) -> Vec<&str> {
        l.words.iter().map(|s| s.as_str()).collect()
    }

    #[test]
    fn words_and_current() {
        let l = p("git checkout ma");
        assert_eq!(w(&l), ["git", "checkout"]);
        assert_eq!((l.cur.as_str(), l.cur_start), ("ma", 13));
        let l = p("git ");
        assert_eq!((w(&l), l.cur.as_str(), l.cur_start), (vec!["git"], "", 4));
    }

    #[test]
    fn quotes_and_escapes() {
        let l = p(r#"echo 'a b' "c \"d\"" e\ f $'g\th' "#);
        assert_eq!(w(&l), ["echo", "a b", "c \"d\"", "e f", "g\th"]);
        let l = p("cat 'my fi");
        assert_eq!((l.cur.as_str(), l.quote, l.cur_start), ("my fi", Some('\''), 4));
        let l = p("cat \"x");
        assert_eq!(l.quote, Some('"'));
        let l = p(r"ls a\ b");
        assert_eq!(l.cur, "a b");
    }

    #[test]
    fn separators() {
        assert_eq!(w(&p("ls | grep foo ")), ["grep", "foo"]);
        assert_eq!(w(&p("a && b || c ; git ")), ["git"]);
        assert_eq!(w(&p("make & git ")), ["git"]);
        assert_eq!(w(&p("a |& git ")), ["git"]);
        assert_eq!(w(&p("a\ngit ")), ["git"]);
        assert_eq!(w(&p("echo 'a | b' ")), ["echo", "a | b"]);
    }

    #[test]
    fn subshells_and_substitution() {
        assert_eq!(w(&p("echo $(git ")), ["git"]);
        assert_eq!(w(&p("echo $(git log) ")), ["echo", ""]); // $(...) is part of a word
        assert_eq!(w(&p("echo \"$(git ")), ["git"]);
        assert_eq!(w(&p("echo `git ")), ["git"]);
        assert_eq!(w(&p("echo `date` x ")), ["echo", "", "x"]);
        assert_eq!(w(&p("(cd foo && git ")), ["git"]);
        let l = p("(cd foo) && git st");
        assert_eq!((w(&l), l.cur.as_str()), (vec!["git"], "st"));
        // After the substitution closes, the outer quote is open again.
        let l = p("echo \"$(date) and");
        assert_eq!((l.quote, l.cur.as_str()), (Some('"'), " and"));
    }

    #[test]
    fn redirections() {
        assert_eq!(w(&p("ls > out.txt -l ")), ["ls", "-l"]);
        assert_eq!(w(&p("ls 2>&1 -a ")), ["ls", "-a"]);
        assert!(p("cat < fi").redirect);
        assert!(!p("cat fi").redirect);
        assert!(p("cat >").redirect);
    }

    #[test]
    fn cursor_in_the_middle_and_multibyte() {
        let s = "git checkout main && ls";
        let l = parse(s, 15); // "git checkout ma|in"
        assert_eq!((w(&l), l.cur.as_str()), (vec!["git", "checkout"], "ma"));
        let s = "echo héllo wörld";
        let l = parse(s, s.len());
        assert_eq!((w(&l), l.cur.as_str()), (vec!["echo", "héllo"], "wörld"));
        assert_eq!(l.cur_start, "echo héllo ".len());
        let l = parse(s, 7); // inside "é": moves back to a char boundary
        assert_eq!(l.cur, "h");
    }

    #[test]
    fn aliases() {
        let al = parse_aliases("alias g=git\nalias gco='git checkout'\nalias ls='ls -G'\nalias -g G='| grep'\n");
        assert_eq!(al.len(), 3);
        let mut l = p("gco ma");
        expand_aliases(&mut l, &al);
        assert_eq!(w(&l), ["git", "checkout"]);
        let mut l = p("ls -");
        expand_aliases(&mut l, &al);
        assert_eq!(w(&l), ["ls", "-G"]); // no loop on ls='ls -G'
        let al = parse_aliases("alias a=b\nalias b=a\n");
        let mut l = p("a x");
        expand_aliases(&mut l, &al);
        assert_eq!(w(&l).len(), 1);
    }

    /// Fuzz: random lines from shell-special characters never panic, and the offsets stay valid.
    #[test]
    fn fuzz() {
        let alphabet: Vec<char> = "ab -'\"\\$()`|&;<>\n\té#{}=~/*".chars().collect();
        let mut x: u64 = 0x9e3779b97f4a7c15;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..200_000 {
            let len = (rnd() % 24) as usize;
            let s: String = (0..len).map(|_| alphabet[(rnd() % alphabet.len() as u64) as usize]).collect();
            let cur = (rnd() as usize) % (s.len() + 1);
            let l = parse(&s, cur);
            assert!(l.cur_start <= s.len() && s.is_char_boundary(l.cur_start), "{s:?} {cur}");
            let mut l2 = parse(&s, s.len());
            expand_aliases(&mut l2, &[("a".into(), "a b".into()), ("b".into(), "a".into())]);
        }
    }
}
