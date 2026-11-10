//! Walks the spec tree with the words before the cursor, then lists what fits the current word.

use crate::generate::{self, Cache, Hit};
use crate::index::{builtin, flag, kind, Index, Node};
use crate::tokenize::Line;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Cmd,
    Sub,
    Opt,
    Arg,
    File,
    Folder,
}

#[derive(Clone, Debug)]
pub struct Cand {
    pub name: String,
    pub desc: String,
    pub kind: Kind,
    pub priority: i32,
    /// 0 prefix, 1 prefix ignoring case, 2 fuzzy (subsequence).
    pub score: u8,
    /// Text to insert, before quoting. `raw` means insert it as is (a spec's insertValue).
    pub body: String,
    pub suffix: &'static str,
    pub raw: bool,
}

#[derive(Default)]
pub struct Out {
    pub cands: Vec<Cand>,
    /// Generators that have no fresh result in the cache. The caller runs them async.
    pub missing: Vec<generate::Req>,
}

pub fn score(name: &str, pat: &str) -> Option<u8> {
    if name.starts_with(pat) {
        return Some(0);
    }
    let (n, p) = (name.to_lowercase(), pat.to_lowercase());
    if n.starts_with(&p) {
        return Some(1);
    }
    let mut it = n.chars();
    p.chars().all(|c| it.any(|x| x == c)).then_some(2)
}

struct Walk<'a> {
    ix: &'a Index,
    cur: &'a str,
    pwd: &'a str,
    cache: &'a Cache,
    out: Out,
}

pub fn complete(ix: &Index, line: &Line, pwd: &str, cache: &Cache, aliases: &[(String, String)]) -> Out {
    let mut w = Walk { ix, cur: &line.cur, pwd, cache, out: Out::default() };
    if line.redirect {
        w.paths(false, &[]);
    } else {
        w.run(&line.words, aliases);
    }
    let mut c = w.out.cands;
    c.sort_by(|a, b| a.score.cmp(&b.score).then(b.priority.cmp(&a.priority)).then_with(|| a.name.cmp(&b.name)));
    c.dedup_by(|a, b| a.name == b.name && a.kind == b.kind);
    // Subsequence matches only when nothing starts with the typed text: "che" lists checkout and
    // cherry-pick, not archive.
    if c.first().is_some_and(|x| x.score < 2) {
        c.retain(|x| x.score < 2);
    }
    w.out.cands = c;
    w.out
}

/// Words like FOO=bar before the command are environment assignments.
fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !k.as_bytes()[0].is_ascii_digit()
    })
}

impl<'a> Walk<'a> {
    fn push(&mut self, name: &str, desc: &str, kind: Kind, priority: i32, body: &str, suffix: &'static str, raw: bool) {
        self.push_any(&[name], desc, kind, priority, body, suffix, raw)
    }

    /// Shows the first name; matches on the best of all names (`co` finds checkout).
    #[allow(clippy::too_many_arguments)]
    fn push_any(&mut self, names: &[&str], desc: &str, kind: Kind, priority: i32, body: &str, suffix: &'static str, raw: bool) {
        // An alias match ranks after main-name matches: `npm ins` is install before ci (install-clean).
        let best = names.iter().enumerate().filter_map(|(i, n)| score(n, self.cur).map(|s| if i == 0 { s } else { s.max(1) })).min();
        let (Some(name), Some(score)) = (names.first(), best) else { return };
        {
            self.out.cands.push(Cand {
                name: name.to_string(),
                desc: desc.to_string(),
                kind,
                priority,
                score,
                body: body.to_string(),
                suffix,
                raw,
            });
        }
    }

    fn run(&mut self, words: &[String], aliases: &[(String, String)]) {
        let mut i = 0;
        while i < words.len() && is_assignment(&words[i]) {
            i += 1;
        }
        // The command word: a spec name. `/usr/bin/git` uses the "git" spec.
        'command: loop {
            if i == words.len() {
                return self.commands(aliases);
            }
            let name = words[i].rsplit('/').next().unwrap_or("");
            let Some(root) = self.ix.root(name) else { return };
            i += 1;
            let mut node = self.load(self.ix.node(root));
            let mut persist: Vec<u32> = Vec::new(); // persistent options of the parent commands
            let mut used: Vec<u32> = Vec::new();
            let mut argi = 0usize;
            let mut pending: Vec<u32> = Vec::new(); // args of the last option, still to fill
            let mut opts_done = false;
            // Parser directives of the command and its parents.
            let mut dirs = node.flags & (flag::POSIX_NC | flag::OPTS_FIRST);
            while i < words.len() {
                let wd = words[i].as_str();
                i += 1;
                if let Some(&a) = pending.first() {
                    if !self.ix.node(a).has(flag::VARIADIC) {
                        pending.remove(0);
                        continue;
                    }
                    if !wd.starts_with('-') || opts_done {
                        continue;
                    }
                    pending.clear(); // a variadic option arg ends at the next option
                }
                if wd == "--" {
                    opts_done = true;
                    continue;
                }
                if wd.starts_with('-') && wd.len() > 1 && !opts_done {
                    let (name, has_val) = match wd.split_once('=') {
                        Some((n, _)) => (n, true),
                        None => (wd, false),
                    };
                    if let Some(o) = self.find_opt(&node, &persist, name) {
                        used.push(o);
                        let on = self.ix.node(o);
                        if !has_val {
                            pending = self.ix.children(&on).filter(|(_, c)| c.kind == kind::ARG).map(|(id, _)| id).collect();
                            if !on.has(flag::REQ_ARG) {
                                // Optional args: keep them only if the next word does not look like an option.
                                if words.get(i).is_some_and(|n| n.starts_with('-')) {
                                    pending.clear();
                                }
                            }
                        }
                    } else if !wd.starts_with("--") && dirs & flag::POSIX_NC == 0 {
                        // Combined short flags: -abc is -a -b -c.
                        for ch in wd[1..].chars() {
                            if let Some(o) = self.find_opt(&node, &persist, &format!("-{ch}")) {
                                used.push(o);
                            }
                        }
                    }
                    continue;
                }
                if argi == 0 {
                    if let Some((_, sub)) = self.ix.children(&node).find(|(_, c)| c.kind == kind::CMD && self.ix.names(c).any(|n| n == wd)) {
                        persist.extend(self.ix.children(&node).filter(|(_, c)| c.kind == kind::OPT && c.has(flag::PERSIST)).map(|(o, _)| o));
                        node = self.load(sub);
                        dirs |= node.flags & (flag::POSIX_NC | flag::OPTS_FIRST);
                        used.clear();
                        continue;
                    }
                }
                if dirs & flag::OPTS_FIRST != 0 {
                    opts_done = true; // `chmod 644 -R`: options only before the first argument
                }
                let args: Vec<(u32, Node)> = self.ix.children(&node).filter(|(_, c)| c.kind == kind::ARG).collect();
                if let Some(&(_, a)) = args.get(argi).or(args.last().filter(|(_, l)| l.has(flag::VARIADIC))) {
                    if a.has(flag::IS_CMD) || a.extra != 0 {
                        // `sudo git ...`: this word is a command; walk its spec from here.
                        // ponytail: an arg loadSpec is treated like isCommand.
                        i -= 1;
                        continue 'command;
                    }
                    if !a.has(flag::VARIADIC) {
                        argi += 1;
                    }
                }
            }
            // The word being typed. An option that needs a value gets only that value.
            if let Some(&a) = pending.first() {
                let an = self.ix.node(a);
                self.arg(&an);
                if !an.has(flag::OPTIONAL) {
                    return;
                }
            }
            let args: Vec<Node> = self.ix.children(&node).filter(|c| c.1.kind == kind::ARG).map(|c| c.1).collect();
            let cur_arg = args.get(argi).or(args.last().filter(|l| l.has(flag::VARIADIC))).copied();
            if self.cur.starts_with('-') && !opts_done {
                self.options(&node, &persist, &used);
                return;
            }
            let before = self.out.cands.len();
            if argi == 0 {
                for (_, c) in self.ix.children(&node) {
                    if c.kind == kind::CMD && !c.has(flag::HIDDEN) {
                        let ins = self.ix.insert_value(&c);
                        let ns: Vec<&str> = self.ix.names(&c).collect();
                        if let Some(&n) = ns.first() {
                            let (body, raw) = if ins.is_empty() { (n, false) } else { (ins, true) };
                            self.push_any(&ns, self.ix.str(c.desc), Kind::Sub, c.priority, body, if raw { "" } else { " " }, raw);
                        }
                    }
                }
            }
            if let Some(a) = cur_arg {
                // `sudo `: any command. `yarn `: the arg's own items (package.json scripts), not PATH.
                let own = a.has(flag::T_FILES | flag::T_FOLDERS) || self.ix.children(&a).next().is_some();
                if a.has(flag::IS_CMD) && !own {
                    return self.commands(aliases);
                }
                self.arg(&a);
            }
            if self.out.cands.len() == before && self.cur.is_empty() {
                self.options(&node, &persist, &used);
            }
            return;
        }
    }

    /// A subcommand with loadSpec "aws/s3" takes its tree from that root.
    fn load(&self, n: Node) -> Node {
        if n.extra != 0 {
            if let Some(r) = self.ix.root(self.ix.str(n.extra)) {
                return self.ix.node(r);
            }
        }
        n
    }

    fn find_opt(&self, node: &Node, persist: &[u32], name: &str) -> Option<u32> {
        self.ix
            .children(node)
            .filter(|(_, c)| c.kind == kind::OPT)
            .map(|(id, _)| id)
            .chain(persist.iter().copied())
            .find(|&o| self.ix.names(&self.ix.node(o)).any(|n| n == name))
    }

    fn options(&mut self, node: &Node, persist: &[u32], used: &[u32]) {
        let used_names: Vec<&str> = used.iter().flat_map(|&u| self.ix.names(&self.ix.node(u)).collect::<Vec<_>>()).collect();
        let ids: Vec<u32> = self.ix.children(node).filter(|(_, c)| c.kind == kind::OPT).map(|(id, _)| id).chain(persist.iter().copied()).collect();
        for id in ids {
            let o = self.ix.node(id);
            if o.has(flag::HIDDEN) || used.contains(&id) && !o.has(flag::REPEAT) {
                continue;
            }
            if self.ix.strs(o.extra).any(|x| used_names.contains(&x)) {
                continue; // exclusiveOn
            }
            let names: Vec<&str> = self.ix.names(&o).collect();
            if used.iter().any(|&u| self.ix.strs(self.ix.node(u).extra).any(|x| names.contains(&x))) {
                continue;
            }
            // Insert the name that matches best; show all names. Options match by prefix with case:
            // `tar -x` must never become `-X`.
            let Some(best) = names.iter().filter_map(|n| n.starts_with(self.cur).then_some((0u8, *n))).min_by_key(|(s, n)| (*s, if self.cur.starts_with("--") { !n.starts_with("--") } else { n.starts_with("--") })) else {
                continue;
            };
            let ins = self.ix.insert_value(&o);
            let suffix = if o.has(flag::REQ_SEP) { "=" } else { " " };
            let (body, sfx, raw) = if ins.is_empty() { (best.1, suffix, false) } else { (ins, "", true) };
            let shown = names.join(", ");
            self.out.cands.push(Cand {
                name: shown,
                desc: self.ix.str(o.desc).to_string(),
                kind: Kind::Opt,
                priority: o.priority,
                score: best.0,
                body: body.to_string(),
                suffix: sfx,
                raw,
            });
        }
    }

    fn arg(&mut self, a: &Node) {
        let filter: Vec<&str> = self.ix.strs(a.names).collect();
        if a.has(flag::T_FILES) {
            self.paths(false, &filter);
        } else if a.has(flag::T_FOLDERS) {
            self.paths(true, &[]);
        }
        let kids: Vec<Node> = self.ix.children(a).map(|c| c.1).collect();
        for c in kids {
            if c.kind == kind::SUG && !c.has(flag::HIDDEN) {
                let k = if c.has(flag::SUG_FOLDER) { Kind::Folder } else if c.has(flag::SUG_FILE) { Kind::File } else { Kind::Arg };
                let ins = self.ix.insert_value(&c);
                let ns: Vec<&str> = self.ix.names(&c).collect();
                if let Some(&n) = ns.first() {
                    let (body, raw) = if ins.is_empty() { (n, false) } else { (ins, true) };
                    self.push_any(&ns, self.ix.str(c.desc), k, c.priority, body, if raw { "" } else { " " }, raw);
                }
            } else if c.kind == kind::GEN {
                let gk = match c.builtin() {
                    0 if c.has(flag::SHELL) => generate::Kind::Shell(self.ix.name(&c).to_string()),
                    0 => generate::Kind::Argv(self.ix.names(&c).map(String::from).collect()),
                    b => generate::Kind::Builtin(b),
                };
                let req = generate::Req::new(gk, self.ix.str(c.extra), self.pwd);
                let items = match self.cache.get(&req.key) {
                    Hit::Fresh(v) => v,
                    Hit::Stale(v) => {
                        self.out.missing.push(req);
                        v
                    }
                    Hit::Miss => {
                        self.out.missing.push(req);
                        continue;
                    }
                };
                // Built-in results before files: Fig set this in the JS postProcess we drop.
                let prio = if c.builtin() != 0 { 75 } else { c.priority };
                let k = match c.builtin() {
                    builtin::GIT_CHANGED => Kind::File,
                    builtin::ZOXIDE => Kind::Folder,
                    _ => Kind::Arg,
                };
                for (i, it) in items.iter().enumerate() {
                    let (name, desc) = it.split_once('\t').unwrap_or((it, ""));
                    // zoxide lists the most used folders first: keep that order.
                    let p = if k == Kind::Folder { prio + (items.len() - i) as i32 } else { prio };
                    self.push(name, desc, k, p, name, " ", false);
                }
            }
        }
    }

    /// Command names: specs, PATH executables and aliases. Only when something is typed.
    fn commands(&mut self, aliases: &[(String, String)]) {
        if self.cur.is_empty() || self.cur.contains('/') {
            if self.cur.contains('/') {
                self.paths(false, &[]);
            }
            return;
        }
        let ix = self.ix;
        for (name, id) in ix.roots() {
            if !name.contains('/') {
                let n = ix.node(id);
                self.push(name, ix.str(n.desc), Kind::Cmd, 60, name, " ", false);
            }
        }
        for (k, v) in aliases {
            self.push(k, v, Kind::Cmd, 55, k, " ", false);
        }
        let req = generate::Req::new(generate::Kind::Builtin(builtin::EXECUTABLES), "", "");
        let items = match self.cache.get(&req.key) {
            Hit::Fresh(v) => Some(v),
            Hit::Stale(v) => {
                self.out.missing.push(req);
                Some(v)
            }
            Hit::Miss => {
                self.out.missing.push(req);
                None
            }
        };
        for it in items.iter().flat_map(|v| v.iter()) {
            self.push(it, "", Kind::Cmd, 50, it, " ", false);
        }
    }

    /// Files and folders for the word being typed: "src/ma" lists "src/" and matches "ma".
    /// `filter` keeps only matching files (".py" is a suffix, else a whole name); folders stay.
    fn paths(&mut self, dirs_only: bool, filter: &[&str]) {
        let cur = self.cur.to_string();
        let (dir, base) = match cur.rfind('/') {
            Some(p) => (&cur[..=p], &cur[p + 1..]),
            None => ("", cur.as_str()),
        };
        let home = std::env::var("HOME").unwrap_or_default();
        let path = if let Some(rest) = dir.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else if dir.starts_with('/') {
            dir.to_string()
        } else {
            format!("{}/{dir}", self.pwd)
        };
        let Ok(rd) = std::fs::read_dir(if path.is_empty() { "/" } else { &path }) else { return };
        let saved = std::mem::replace(&mut self.cur, "");
        let start = self.out.cands.len();
        // Filter first, then cap: in a folder of 1,500 files, `rep` must still find report.pdf.
        // ponytail: reads at most 20,000 entries per key; a bigger folder shows a partial list.
        for e in rd.flatten().take(20_000) {
            if self.out.cands.len() - start >= 1000 {
                break;
            }
            let Ok(name) = e.file_name().into_string() else { continue };
            if name.starts_with('.') && !base.starts_with('.') {
                continue;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink() && e.path().is_dir());
            if !is_dir && (dirs_only || !filter.is_empty() && !matches_filter(&name, filter)) {
                continue;
            }
            let Some(s) = score(&name, base) else { continue };
            let body = format!("{dir}{name}");
            self.out.cands.push(Cand {
                name: if is_dir { format!("{name}/") } else { name },
                desc: String::new(),
                kind: if is_dir { Kind::Folder } else { Kind::File },
                priority: 50,
                score: s,
                body,
                suffix: if is_dir { "/" } else { " " },
                raw: false,
            });
        }
        self.cur = saved;
    }
}

fn matches_filter(name: &str, filter: &[&str]) -> bool {
    let low = name.to_lowercase();
    filter.iter().any(|f| if f.starts_with('.') { low.ends_with(&f.to_lowercase()) } else { name == *f })
}

/// Marks where the cursor goes after an insert (an insertValue's `{cursor}`).
pub const CURSOR: char = '\x01';

/// The text to insert for a candidate: quoted for the shell, or inside the open quote.
pub fn insert_text(c: &Cand, quote: Option<char>) -> String {
    if c.raw {
        // `-m '{cursor}'`: the cursor goes between the quotes. At the end it needs no mark.
        let b = c.body.replacen("{cursor}", &CURSOR.to_string(), 1).replace("{cursor}", "");
        return b.strip_suffix(CURSOR).map_or(b.clone(), String::from);
    }
    match quote {
        Some(q) => {
            // Inside '…' (and $'…') a quote is '\''. Inside "…", $ ` " \ need a backslash.
            let body = if q == '\'' {
                c.body.replace('\'', "'\\''")
            } else {
                c.body.chars().flat_map(|ch| if "$`\"\\".contains(ch) { vec!['\\', ch] } else { vec![ch] }).collect()
            };
            if c.suffix == " " { format!("{q}{body}{q} ") } else { format!("{q}{body}{}", c.suffix) }
        }
        // Tab, newline and other control characters only work inside $'…'.
        None if c.body.chars().any(|ch| ch.is_control()) => {
            let mut s = String::from("$'");
            for ch in c.body.chars() {
                match ch {
                    '\\' | '\'' => {
                        s.push('\\');
                        s.push(ch);
                    }
                    '\t' => s.push_str("\\t"),
                    '\n' => s.push_str("\\n"),
                    ch if ch.is_control() => s.push_str(&format!("\\x{:02x}", ch as u32)),
                    ch => s.push(ch),
                }
            }
            s.push('\'');
            s.push_str(c.suffix);
            s
        }
        None => {
            let mut s = String::with_capacity(c.body.len() + 2);
            for (i, ch) in c.body.chars().enumerate() {
                // A leading ~ (tilde expansion) or = (command path expansion) is literal in a
                // file name. A typed "~/" prefix stays: the body then has a slash.
                let lead = i == 0 && (ch == '=' || ch == '~' && !c.body.contains('/'));
                if lead || " !\"#$&'()*;<>?[\\]^`{|}".contains(ch) {
                    s.push('\\');
                }
                s.push(ch);
            }
            s.push_str(c.suffix);
            s
        }
    }
}

/// Golden tests: specs in (Fig-style JSON, through the compiler), index lookups out.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, generate, index, tokenize};

    fn index() -> Index {
        let mut w = index::Writer::default();
        let git: serde_json::Value = serde_json::from_str(include_str!("../specs/git.json")).unwrap();
        compile::add_root(&mut w, "git", &git);
        let more = [
            ("sudo", r#"{"name":"sudo","options":[{"name":"-u","args":{"name":"user"}}],"args":{"name":"command","isCommand":true}}"#),
            ("runx", r#"{"name":"runx","args":{"name":"script","isCommand":true,"suggestions":["alpha"]}}"#),
            ("aws", r#"{"name":"aws","subcommands":[{"name":"s3","loadSpec":"aws/s3"}]}"#),
            ("aws/s3", r#"{"name":"s3","subcommands":[{"name":"cp","description":"Copy"},{"name":"ls"}]}"#),
            ("tool", r#"{"name":"tool","options":[
                {"name":"--out","requiresSeparator":true,"args":{"name":"f"}},
                {"name":"--secret","hidden":true},
                {"name":"--yes","insertValue":"--yes {cursor}"},
                {"name":"-e","insertValue":"-e '{cursor}'"}],
              "args":{"name":"x","suggestions":["alpha",{"name":"beta","description":"B"}],
                      "generators":{"script":"printf 'gen1\\ngen2'","splitOn":"\n"}}}"#),
            ("py", r#"{"name":"py","args":{"generators":{"template":"filepaths","extensions":["py"],"equals":["Pipfile"]}}}"#),
            ("ch", r#"{"name":"ch","parserDirectives":{"optionsMustPrecedeArguments":true},"options":[{"name":"-R"}],"args":{"name":"f","isVariadic":true}}"#),
            ("find", r#"{"name":"find","parserDirectives":{"flagsArePosixNoncompliant":true},"options":[{"name":"-n"},{"name":"-name","args":{"name":"p"}}]}"#),
            ("tar", r#"{"name":"tar","options":[{"name":"-x"},{"name":"-X"},{"name":"--index-file"}]}"#),
        ];
        for (name, j) in more {
            compile::add_root(&mut w, name, &serde_json::from_str(j).unwrap());
        }
        Index::from_bytes(w.finish()).unwrap()
    }

    /// Names in rank order, after running the generators once (as the async path does).
    fn names(ix: &Index, buf: &str) -> Vec<String> {
        let mut cache = generate::Cache::default();
        let line = tokenize::parse(buf, buf.len());
        for g in complete(ix, &line, "/", &cache, &[]).missing {
            let v = generate::run(&g);
            cache.put(g.key, v);
        }
        complete(ix, &line, "/", &cache, &[]).cands.into_iter().map(|c| c.name).collect()
    }

    #[test]
    fn subcommands_rank_prefix_then_priority_then_name() {
        let ix = index();
        let n = names(&ix, "git ");
        assert_eq!(&n[..4], ["status", "add", "checkout", "commit"]); // priority 80, then 70s by name
        let n = names(&ix, "git co");
        assert_eq!(&n[..2], ["commit", "checkout"]); // "co" is an alias of checkout: after the name match
        assert!(n.iter().skip(2).all(|x| !x.starts_with("co"))); // the rest are fuzzy matches
    }

    #[test]
    fn options_used_exclusive_and_persistent() {
        let ix = index();
        let n = names(&ix, "git branch -a -");
        assert!(!n.iter().any(|x| x.contains("--remotes")), "exclusiveOn: {n:?}");
        assert!(!n.iter().any(|x| x == "-a, --all"), "already used: {n:?}");
        assert!(n.iter().any(|x| x == "-C"), "persistent option of git: {n:?}");
        let n = names(&ix, "git commit -m x -");
        assert!(n.iter().any(|x| x == "-m, --message"), "isRepeatable: {n:?}");
        // An option that takes a value: the value gets no subcommands or options.
        assert!(names(&ix, "git commit -m ").is_empty());
    }

    #[test]
    fn load_spec_and_is_command() {
        let ix = index();
        assert_eq!(names(&ix, "aws s3 c")[0], "cp");
        assert_eq!(names(&ix, "sudo -u root git sta")[0], "status");
        assert_eq!(names(&ix, "FOO=1 /usr/bin/git stat")[0], "status");
        // An isCommand arg with its own items (yarn scripts) shows them, not the PATH commands.
        assert_eq!(names(&ix, "runx "), ["alpha"]);
    }

    #[test]
    fn suggestions_generators_hidden_insert_values() {
        let ix = index();
        let n = names(&ix, "tool ");
        assert_eq!(n, ["alpha", "beta", "gen1", "gen2"]);
        let line = tokenize::parse("tool --", 7);
        let c = complete(&ix, &line, "/", &generate::Cache::default(), &[]).cands;
        let get = |n: &str| c.iter().find(|x| x.name == n).unwrap();
        assert!(c.iter().all(|x| x.name != "--secret"));
        assert_eq!(insert_text(get("--out"), None), "--out=");
        assert_eq!(insert_text(get("--yes"), None), "--yes ");
        let e = complete(&ix, &tokenize::parse("tool -e", 7), "/", &generate::Cache::default(), &[]).cands;
        assert_eq!(insert_text(&e[0], None), "-e '\x01'"); // the cursor goes between the quotes
    }

    #[test]
    fn quoting_of_inserted_names() {
        let f = |body: &str, suffix: &'static str| Cand {
            name: body.into(),
            desc: String::new(),
            kind: Kind::File,
            priority: 0,
            score: 0,
            body: body.into(),
            suffix,
            raw: false,
        };
        assert_eq!(insert_text(&f("it's", "/"), Some('\'')), "'it'\\''s/");
        assert_eq!(insert_text(&f("a$b.txt", " "), Some('"')), "\"a\\$b.txt\" ");
        assert_eq!(insert_text(&f("~home", " "), None), "\\~home ");
        assert_eq!(insert_text(&f("~/Desktop", "/"), None), "~/Desktop/"); // typed ~/ stays
        assert_eq!(insert_text(&f("=x", " "), None), "\\=x ");
        assert_eq!(insert_text(&f("a\tb", " "), None), "$'a\\tb' ");
        assert_eq!(insert_text(&f("my file", " "), None), "my\\ file ");
    }

    #[test]
    fn big_folders_filter_before_the_cap() {
        let dir = std::env::temp_dir().join(format!("hk-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..1500 {
            std::fs::write(dir.join(format!("f{i:04}")), "").unwrap();
        }
        std::fs::write(dir.join("report.pdf"), "").unwrap();
        let ix = index();
        let line = tokenize::parse("git add rep", 11);
        let c = complete(&ix, &line, dir.to_str().unwrap(), &generate::Cache::default(), &[]).cands;
        assert_eq!(c.first().map(|x| x.name.as_str()), Some("report.pdf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_filters_directives_and_matching() {
        let dir = std::env::temp_dir().join(format!("hk-filter-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.py", "B.PY", "b.txt", "Pipfile"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let ix = index();
        let c = |buf: &str| -> Vec<String> {
            complete(&ix, &tokenize::parse(buf, buf.len()), dir.to_str().unwrap(), &generate::Cache::default(), &[]).cands.into_iter().map(|c| c.name).collect()
        };
        assert_eq!(c("py "), ["B.PY", "Pipfile", "a.py", "sub/"]); // extensions, equals, folders
        assert!(c("ch -").contains(&"-R".to_string()));
        assert!(c("ch x -").is_empty(), "optionsMustPrecedeArguments");
        assert_eq!(c("tar -x"), ["-x"]); // never -X or a subsequence
        assert!(c("git che").iter().all(|n| n.starts_with("che")), "no fuzzy when a prefix matches");
        assert!(!c("git chkt").is_empty(), "fuzzy when nothing matches by prefix");
        // -name is one option, so -n is still unused after it.
        assert!(c("find -name x -").contains(&"-n".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_folders_quoting_and_aliases() {
        let dir = std::env::temp_dir().join(format!("hk-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("my dir")).unwrap();
        std::fs::write(dir.join("a file.txt"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        let ix = index();
        let pwd = dir.to_str().unwrap();
        let c = |buf: &str| complete(&ix, &tokenize::parse(buf, buf.len()), pwd, &generate::Cache::default(), &[]).cands;
        let r = c("git add ");
        assert_eq!(r.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a file.txt", "my dir/"]);
        assert_eq!(insert_text(&r[1], None), "my\\ dir/");
        assert_eq!(insert_text(&r[0], Some('\'')), "'a file.txt' ");
        assert_eq!(c("git add .h")[0].name, ".hidden");
        assert_eq!(c("git -C ")[0].name, "my dir/"); // folders only
        let al = tokenize::parse_aliases("alias gco='git checkout'");
        let mut line = tokenize::parse("gco -", 5);
        tokenize::expand_aliases(&mut line, &al);
        assert!(complete(&ix, &line, pwd, &generate::Cache::default(), &al).cands.iter().any(|x| x.name == "-b"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn real_index_lookups() {
        // The compiled Fig index, if it was built (make index). Skipped otherwise.
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("build/specs.idx");
        let Ok(ix) = Index::open(&p) else { return };
        assert!(names(&ix, "docker run --r").contains(&"--rm".to_string()));
        assert_eq!(names(&ix, "npm ins")[0], "install");
        assert_eq!(names(&ix, "brew install --cas")[0].split(',').next().unwrap(), "--cask");
    }
}
