//! Generators: built-in ones (git, npm scripts, make targets, ssh hosts, zoxide, PATH) and script
//! ones from specs. Each run has a 200 ms timeout, runs in the request's PWD, and is cached with a
//! short TTL. An item is "name" or "name\tdescription".

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::index::builtin;

pub const TIMEOUT: Duration = Duration::from_millis(200);
/// Results older than this are still served, but a refresh runs in the background.
pub const TTL: Duration = Duration::from_secs(5);
const MAX_KEYS: usize = 32;
const MAX_ITEMS: usize = 1000;
const MAX_OUTPUT: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Builtin(u32),
    Argv(Vec<String>),
    Shell(String),
}

#[derive(Clone, Debug)]
pub struct Req {
    pub key: String,
    pub kind: Kind,
    pub split: String,
    pub pwd: String,
}

impl Req {
    pub fn new(kind: Kind, split: &str, pwd: &str) -> Req {
        let what = match &kind {
            Kind::Builtin(b) => format!("b{b}"),
            Kind::Argv(a) => format!("a{}", a.join("\x1f")),
            Kind::Shell(s) => format!("s{s}"),
        };
        // PATH, ssh hosts and zoxide do not depend on the folder. Git refs depend on HEAD, the
        // changed files on the git index (edits show up one TTL later).
        let place = match kind {
            Kind::Builtin(builtin::EXECUTABLES | builtin::SSH_HOSTS | builtin::ZOXIDE) => String::new(),
            Kind::Builtin(builtin::GIT_CHANGED) => format!("{pwd}\0{:?}", git_mtime(Path::new(pwd), "index")),
            Kind::Builtin(builtin::GIT_BRANCHES | builtin::GIT_REMOTES | builtin::GIT_TAGS) => {
                format!("{pwd}\0{:?}", git_mtime(Path::new(pwd), "HEAD"))
            }
            _ => pwd.to_string(),
        };
        Req { key: format!("{what}\0{split}\0{place}"), kind, split: split.to_string(), pwd: pwd.to_string() }
    }
}

fn git_mtime(dir: &Path, file: &str) -> Option<SystemTime> {
    let mut d = Some(dir);
    while let Some(p) = d {
        let dot = p.join(".git");
        if dot.is_dir() {
            return std::fs::metadata(dot.join(file)).and_then(|m| m.modified()).ok();
        }
        if dot.is_file() {
            // A worktree: ".git" holds "gitdir: <path>".
            let s = std::fs::read_to_string(&dot).ok()?;
            let g = PathBuf::from(s.trim().strip_prefix("gitdir: ")?);
            return std::fs::metadata(p.join(g).join(file)).and_then(|m| m.modified()).ok();
        }
        d = p.parent();
    }
    None
}

#[derive(Default)]
pub struct Cache {
    map: HashMap<String, (Arc<Vec<String>>, Instant)>,
}

pub enum Hit {
    Fresh(Arc<Vec<String>>),
    Stale(Arc<Vec<String>>),
    Miss,
}

impl Cache {
    pub fn get(&self, key: &str) -> Hit {
        match self.map.get(key) {
            Some((v, t)) if t.elapsed() < TTL => Hit::Fresh(v.clone()),
            Some((v, _)) => Hit::Stale(v.clone()),
            None => Hit::Miss,
        }
    }
    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
    pub fn put(&mut self, key: String, v: Vec<String>) {
        if self.map.len() >= MAX_KEYS && !self.map.contains_key(&key) {
            let oldest = self.map.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.map.remove(&k);
            }
        }
        self.map.insert(key, (Arc::new(v), Instant::now()));
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Runs the generator. Blocks for at most TIMEOUT (except PATH scanning, which runs no command).
pub fn run(r: &Req) -> Vec<String> {
    let lines = |out: Option<Vec<u8>>, split: &str| -> Vec<String> {
        let s = String::from_utf8_lossy(&out.unwrap_or_default()).into_owned();
        let sep = if split.is_empty() { "\n" } else { split };
        s.split(sep).map(|x| x.trim()).filter(|x| !x.is_empty()).take(MAX_ITEMS).map(String::from).collect()
    };
    let git = |args: &[&str]| {
        let mut c = Command::new("git");
        c.arg("--no-optional-locks").args(args).env("GIT_OPTIONAL_LOCKS", "0");
        lines(output(c, &r.pwd), "\n")
    };
    match &r.kind {
        Kind::Builtin(builtin::GIT_BRANCHES) => {
            let refs = git(&["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"]);
            refs.iter()
                .filter(|r| !r.ends_with("/HEAD"))
                .filter_map(|r| r.strip_prefix("refs/heads/").or_else(|| r.strip_prefix("refs/remotes/")))
                .map(String::from)
                .collect()
        }
        Kind::Builtin(builtin::GIT_REMOTES) => git(&["remote"]),
        Kind::Builtin(builtin::GIT_TAGS) => git(&["tag", "--sort=-creatordate"]),
        Kind::Builtin(builtin::GIT_CHANGED) => {
            git(&["status", "--short", "--untracked-files=normal"]).iter().filter_map(|l| changed_path(l)).collect()
        }
        Kind::Builtin(builtin::EXECUTABLES) => executables(),
        Kind::Builtin(builtin::NPM_SCRIPTS) => npm_scripts(Path::new(&r.pwd)),
        Kind::Builtin(builtin::MAKE_TARGETS) => make_targets(Path::new(&r.pwd)),
        Kind::Builtin(builtin::SSH_HOSTS) => {
            let home = std::env::var("HOME").unwrap_or_default();
            let read = |f: &str| std::fs::read_to_string(format!("{home}/.ssh/{f}")).unwrap_or_default();
            ssh_hosts(&read("config"), &read("known_hosts"))
        }
        Kind::Builtin(builtin::ZOXIDE) => {
            let mut c = Command::new("zoxide");
            c.args(["query", "--list"]);
            let home = std::env::var("HOME").unwrap_or_default();
            lines(output(c, "/"), "\n")
                .into_iter()
                .map(|p| match p.strip_prefix(&home) {
                    Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
                    _ => p,
                })
                .collect()
        }
        Kind::Builtin(_) => vec![],
        Kind::Argv(a) if !a.is_empty() => {
            let mut c = Command::new(&a[0]);
            c.args(&a[1..]);
            lines(output(c, &r.pwd), &r.split)
        }
        Kind::Argv(_) => vec![],
        Kind::Shell(s) => {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(s);
            lines(output(c, &r.pwd), &r.split)
        }
    }
}

/// One line of `git status --short`: "XY path", "R  old -> new", or a quoted path. Paths are
/// relative to the current folder.
fn changed_path(l: &str) -> Option<String> {
    let (_, p) = l.trim_start().split_once(' ')?;
    let p = p.trim_start();
    let p = p.rsplit_once(" -> ").map_or(p, |x| x.1);
    // ponytail: a quoted path ("a\tb") keeps its escapes; unescape if such names show up.
    Some(p.trim_matches('"').to_string()).filter(|p| !p.is_empty())
}

/// The "scripts" of the nearest package.json, with the command as the description.
fn npm_scripts(dir: &Path) -> Vec<String> {
    let Some(text) = dir.ancestors().find_map(|d| std::fs::read_to_string(d.join("package.json")).ok()) else {
        return vec![];
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let Some(o) = v.get("scripts").and_then(|s| s.as_object()) else { return vec![] };
    o.iter().map(|(k, c)| format!("{k}\t{}", c.as_str().unwrap_or("").replace(['\t', '\n'], " "))).take(MAX_ITEMS).collect()
}

/// Targets of the Makefile in `dir`: "name:" lines, not variables, patterns or special targets.
/// A target with a `/` is a file the Makefile builds (build/specs.idx), not one to type: skip it.
/// A "## text" comment after the target is its description.
fn make_targets(dir: &Path) -> Vec<String> {
    let Some(text) = ["GNUmakefile", "makefile", "Makefile"].iter().find_map(|f| std::fs::read_to_string(dir.join(f)).ok()) else {
        return vec![];
    };
    let mut v = Vec::new();
    for l in text.lines() {
        if l.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some((t, rest)) = l.split_once(':') else { continue };
        if rest.starts_with([':', '=']) || t.contains(['=', '$', '%']) {
            continue;
        }
        let desc = rest.split_once("##").map_or("", |x| x.1.trim());
        for name in t.split_whitespace().filter(|n| !n.contains('/')) {
            v.push(if desc.is_empty() { name.to_string() } else { format!("{name}\t{desc}") });
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Hosts from ~/.ssh/config (`Host` lines, no patterns) and ~/.ssh/known_hosts (not hashed).
/// ponytail: `Include` files are not read.
fn ssh_hosts(config: &str, known: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for l in config.lines() {
        let l = l.trim();
        let Some((k, rest)) = l.split_once(|c: char| c.is_whitespace() || c == '=') else { continue };
        if k.eq_ignore_ascii_case("host") {
            v.extend(rest.split_whitespace().filter(|h| !h.contains(['*', '?', '!'])).map(String::from));
        }
    }
    for l in known.lines() {
        let Some(f) = l.split_whitespace().next() else { continue };
        if f.starts_with(['|', '#', '@']) {
            continue;
        }
        for h in f.split(',') {
            // [host]:2222 is a host on another port.
            let h = h.strip_prefix('[').and_then(|x| x.split_once("]:")).map_or(h, |x| x.0);
            v.push(h.to_string());
        }
    }
    v.sort();
    v.dedup();
    v
}

fn executables() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        // ponytail: no stat per entry (PATH dirs hold executables); stat if non-executables show up.
        v.extend(rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')));
    }
    v.sort();
    v.dedup();
    v
}

/// Runs a command in `pwd` with a timeout. Returns None on timeout or failure to start.
fn output(mut c: Command, pwd: &str) -> Option<Vec<u8>> {
    c.current_dir(if pwd.is_empty() { "/" } else { pwd })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn().ok()?;
    let mut out = child.stdout.take()?;
    unsafe {
        let fd = out.as_raw_fd();
        libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + TIMEOUT;
    let (mut buf, mut tmp, mut eof) = (Vec::new(), [0u8; 8192], false);
    loop {
        if !eof {
            match out.read(&mut tmp) {
                Ok(0) => eof = true,
                Ok(n) if buf.len() < MAX_OUTPUT => {
                    buf.extend_from_slice(&tmp[..n]);
                    continue;
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => eof = true,
            }
        }
        if eof && matches!(child.try_wait(), Ok(Some(_))) {
            return Some(buf);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_runs_in_pwd_and_splits() {
        let r = Req::new(Kind::Shell("printf 'a,b,,c'".into()), ",", "/");
        assert_eq!(run(&r), ["a", "b", "c"]);
        let r = Req::new(Kind::Argv(vec!["pwd".into()]), "\n", "/tmp");
        assert!(run(&r)[0].ends_with("tmp"));
    }

    #[test]
    fn timeout_kills_slow_commands() {
        let t = Instant::now();
        let r = Req::new(Kind::Shell("sleep 5".into()), "\n", "/");
        assert!(run(&r).is_empty());
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn builtin_parsers() {
        assert_eq!(changed_path(" M src/a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(changed_path("?? new dir/").as_deref(), Some("new dir/"));
        assert_eq!(changed_path("R  old.rs -> new.rs").as_deref(), Some("new.rs"));
        assert_eq!(changed_path("MM \"a b\"").as_deref(), Some("a b"));
        let hosts = ssh_hosts("Host web db\n  HostName x\nHost *.corp !bad\nhost=box\n", "gh.com,1.2.3.4 ssh-ed25519 K\n|1|hash= ssh-rsa K\n[jump]:2222 ssh-rsa K\n");
        assert_eq!(hosts, ["1.2.3.4", "box", "db", "gh.com", "jump", "web"]);
        let d = std::env::temp_dir().join(format!("hk-gen-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("package.json"), r#"{"scripts":{"build":"tsc","dev":"vite"}}"#).unwrap();
        assert_eq!(npm_scripts(&d.join("sub")), ["build\ttsc", "dev\tvite"]); // found in a parent folder
        std::fs::write(d.join("Makefile"), "X := 1\nall: build ## Build it\nbuild test:\n\techo\n.PHONY: all\n%.o: %.c\n$(BIN): x\nY ?= 2\nout/a.idx: x\n").unwrap();
        assert_eq!(make_targets(&d), ["all\tBuild it", "build", "test"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn cache_is_capped() {
        let mut c = Cache::default();
        for i in 0..100 {
            c.put(format!("k{i}"), vec![]);
        }
        assert_eq!(c.len(), MAX_KEYS);
        assert!(matches!(c.get("k99"), Hit::Fresh(_)));
    }
}
