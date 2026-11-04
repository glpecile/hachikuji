//! Generators: built-in ones (git refs, PATH executables) and script ones from specs.
//! Each run has a 200 ms timeout, runs in the request's PWD, and is cached with a short TTL.

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
        // PATH executables do not depend on the folder. Git refs also depend on HEAD.
        let place = match kind {
            Kind::Builtin(builtin::EXECUTABLES) => String::new(),
            Kind::Builtin(_) => format!("{pwd}\0{:?}", git_head_mtime(Path::new(pwd))),
            _ => pwd.to_string(),
        };
        Req { key: format!("{what}\0{split}\0{place}"), kind, split: split.to_string(), pwd: pwd.to_string() }
    }
}

fn git_head_mtime(dir: &Path) -> Option<SystemTime> {
    let mut d = Some(dir);
    while let Some(p) = d {
        let dot = p.join(".git");
        if dot.is_dir() {
            return std::fs::metadata(dot.join("HEAD")).and_then(|m| m.modified()).ok();
        }
        if dot.is_file() {
            // A worktree: ".git" holds "gitdir: <path>".
            let s = std::fs::read_to_string(&dot).ok()?;
            let g = PathBuf::from(s.trim().strip_prefix("gitdir: ")?);
            return std::fs::metadata(p.join(g).join("HEAD")).and_then(|m| m.modified()).ok();
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
        Kind::Builtin(builtin::EXECUTABLES) => executables(),
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
    fn cache_is_capped() {
        let mut c = Cache::default();
        for i in 0..100 {
            c.put(format!("k{i}"), vec![]);
        }
        assert_eq!(c.len(), MAX_KEYS);
        assert!(matches!(c.get("k99"), Hit::Fresh(_)));
    }
}
