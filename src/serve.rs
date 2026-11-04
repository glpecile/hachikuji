//! The helper process: one per shell. Reads requests from a FIFO, writes responses to another.
//!
//! Request frame: NUL-terminated fields.
//!   "1" "q" seq cursor(bytes) pwd columns lines selected buffer
//!   "1" "a" aliases(`alias -L` output)
//! Response frame: fields separated by 0x1f, ended by 0x1e.
//!   "1" seq "r" count selected replace_chars insert popup_text highlights("\n"-separated)
//!   "1" "0" "h" pid            (hello, sent once at start)
//! A response for the same seq can come again when an async generator finishes.

use crate::complete::{self, insert_text};
use crate::generate::{self, Cache};
use crate::index::Index;
use crate::render::{self, Config};
use crate::tokenize;
use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const VERSION: &str = "1";
const MAX_JOBS: usize = 4;

pub struct Engine {
    pub ix: Index,
    pub cfg: Config,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub seq: u64,
    pub cursor: usize,
    pub pwd: String,
    pub cols: usize,
    pub lines: usize,
    pub sel: usize,
    pub buffer: String,
}

#[derive(Default)]
pub struct Shared {
    pub cache: Cache,
    pub aliases: Vec<(String, String)>,
    last: Option<Request>,
    running: HashSet<String>,
}

/// Builds the response frame for a request. Also returns the generators it still needs.
pub fn respond(e: &Engine, s: &Shared, r: &Request, out: &mut Vec<u8>) -> Vec<generate::Req> {
    let mut line = tokenize::parse(&r.buffer, r.cursor);
    tokenize::expand_aliases(&mut line, &s.aliases);
    let res = complete::complete(&e.ix, &line, &r.pwd, &s.cache, &s.aliases);
    let cursor = r.cursor.min(r.buffer.len());
    let replace = r.buffer.get(line.cur_start..cursor).map_or(0, |t| t.chars().count());
    let popup = render::render(&res.cands, r.sel, r.cols, r.lines, &e.cfg);
    out.clear();
    let mut f = |x: &str| {
        out.extend_from_slice(x.as_bytes());
        out.push(0x1f);
    };
    f(VERSION);
    f(&r.seq.to_string());
    f("r");
    match &popup {
        Some(p) => {
            f(&res.cands.len().to_string());
            f(&p.sel.to_string());
            f(&replace.to_string());
            f(&insert_text(&res.cands[p.sel], line.quote));
            f(&p.text);
            f(&p.hl.join("\n"));
        }
        None => ["0", "0", "0", "", "", ""].iter().for_each(|x| f(x)),
    }
    out.pop();
    out.push(0x1e);
    res.missing
}

/// Reads one NUL-terminated field. None on EOF.
fn field(r: &mut impl BufRead, buf: &mut Vec<u8>) -> Option<String> {
    buf.clear();
    match r.read_until(0, buf) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            if buf.last() == Some(&0) {
                buf.pop();
            }
            Some(String::from_utf8_lossy(buf).into_owned())
        }
    }
}

pub fn parse_request(r: &mut impl BufRead, buf: &mut Vec<u8>) -> Option<Result<Request, String>> {
    if field(r, buf)? != VERSION {
        return None; // another protocol version: exit, the shell restarts a matching helper
    }
    match field(r, buf)?.as_str() {
        "q" => {
            let mut n = || field(r, buf);
            let (seq, cursor, pwd, cols, lines, sel) = (n()?, n()?, n()?, n()?, n()?, n()?);
            let buffer = n()?;
            Some(Ok(Request {
                seq: seq.parse().unwrap_or(0),
                cursor: cursor.parse().unwrap_or(0),
                pwd,
                cols: cols.parse().unwrap_or(80),
                lines: lines.parse().unwrap_or(24),
                sel: sel.parse().unwrap_or(0),
                buffer,
            }))
        }
        "a" => Some(Err(field(r, buf)?)),
        _ => None,
    }
}

pub fn index_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let exe = std::env::current_exe().ok();
    let dir = exe.as_ref().and_then(|e| e.parent());
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("HACHIKUJI_INDEX") {
        c.push(p.into());
    }
    c.push(format!("{home}/.cache/hachikuji/specs.idx").into());
    if let Some(d) = dir {
        c.push(d.parent().unwrap_or(d).join("share/hachikuji/specs.idx"));
        c.push(d.join("specs.idx"));
    }
    c.into_iter().find(|p| p.is_file())
}

pub fn load_index() -> Index {
    index_path().and_then(|p| Index::open(&p).ok()).unwrap_or_else(Index::empty)
}

/// Leaves the terminal's session and process groups: Ctrl-C never reaches the helper, and
/// tools that read the folder of the foreground process see the shell.
fn detach() {
    unsafe {
        if libc::setsid() < 0 {
            // zsh job control made us a process group leader, and a leader cannot call setsid.
            match libc::fork() {
                0 => {
                    libc::setsid();
                }
                -1 => {}
                _ => libc::_exit(0),
            }
        }
        for s in [libc::SIGINT, libc::SIGTSTP, libc::SIGHUP, libc::SIGPIPE, libc::SIGQUIT, libc::SIGTTIN, libc::SIGTTOU] {
            libc::signal(s, libc::SIG_IGN);
        }
        // Close what the shell let us inherit (other plugins' pipes): holding them open would
        // hide EOF from their readers.
        for fd in 3..1024 {
            libc::close(fd);
        }
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            for fd in 0..3 {
                libc::dup2(null, fd);
            }
        }
        libc::chdir(c"/".as_ptr());
    }
}

type Out = Arc<Mutex<std::fs::File>>;

fn send(out: &Out, frame: &[u8]) {
    let mut o = out.lock().unwrap();
    if o.write_all(frame).is_err() {
        std::process::exit(0); // EPIPE: the shell is gone
    }
}

fn spawn_jobs(missing: Vec<generate::Req>, s: &mut Shared, e: &Arc<Engine>, sh: &Arc<Mutex<Shared>>, out: &Out) {
    if std::env::var_os("HACHIKUJI_NO_GENERATORS").is_some() {
        return; // latency tests of static specs only
    }
    for g in missing {
        if s.running.len() >= MAX_JOBS || !s.running.insert(g.key.clone()) {
            continue;
        }
        let (e, sh, out) = (e.clone(), sh.clone(), out.clone());
        let ok = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let items = generate::run(&g);
            let mut s = sh.lock().unwrap();
            s.running.remove(&g.key);
            s.cache.put(g.key, items);
            if let Some(r) = s.last.clone() {
                let mut frame = Vec::new();
                let more = respond(&e, &s, &r, &mut frame);
                send(&out, &frame);
                spawn_jobs(more, &mut s, &e, &sh, &out);
            }
        });
        if ok.is_err() {
            s.running.clear();
        }
    }
}

pub fn serve(dir: &Path) -> io::Result<()> {
    detach();
    // Non-blocking opens: if the shell is already gone, a blocking open would wait forever.
    // The read side then gets EOF at once, and the write side fails with ENXIO.
    use std::os::unix::fs::OpenOptionsExt;
    let open = |name: &str, write: bool| -> io::Result<std::fs::File> {
        let f = std::fs::OpenOptions::new().read(!write).write(write).custom_flags(libc::O_NONBLOCK).open(dir.join(name))?;
        unsafe {
            use std::os::unix::io::AsRawFd;
            let fd = f.as_raw_fd();
            libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK);
        }
        Ok(f)
    };
    let q = open("q", false)?;
    let r = open("r", true)?;
    let _ = std::fs::remove_file(dir.join("q"));
    let _ = std::fs::remove_file(dir.join("r"));
    let _ = std::fs::remove_dir(dir);
    let out: Out = Arc::new(Mutex::new(r));
    send(&out, format!("{VERSION}\x1f0\x1fh\x1f{}\x1e", std::process::id()).as_bytes());

    let e = Arc::new(Engine { ix: load_index(), cfg: Config::load() });
    let sh = Arc::new(Mutex::new(Shared::default()));
    let mut rd = BufReader::new(q);
    let (mut buf, mut frame) = (Vec::new(), Vec::new());
    loop {
        match parse_request(&mut rd, &mut buf) {
            None => return Ok(()), // EOF: the shell is gone
            Some(Err(aliases)) => sh.lock().unwrap().aliases = tokenize::parse_aliases(&aliases),
            Some(Ok(req)) => {
                let mut s = sh.lock().unwrap();
                let missing = respond(&e, &s, &req, &mut frame);
                s.last = Some(req);
                send(&out, &frame);
                spawn_jobs(missing, &mut s, &e, &sh, &out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let raw = b"1\0a\0alias g=git\x001\0q\x007\x004\0/tmp\x0080\x0024\x001\0g st\0";
        let mut r = std::io::Cursor::new(&raw[..]);
        let mut b = Vec::new();
        assert_eq!(parse_request(&mut r, &mut b), Some(Err("alias g=git".into())));
        let q = parse_request(&mut r, &mut b).unwrap().unwrap();
        assert_eq!((q.seq, q.cursor, q.pwd.as_str(), q.sel, q.buffer.as_str()), (7, 4, "/tmp", 1, "g st"));
        assert!(parse_request(&mut r, &mut b).is_none());
    }
}
