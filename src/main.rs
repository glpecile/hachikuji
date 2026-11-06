mod compile;
mod complete;
mod generate;
mod index;
mod render;
mod serve;
mod tokenize;

use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "usage:
  hachikuji init zsh                 print the line to add to ~/.zshrc
  hachikuji serve <dir>              run the helper (the shell starts it)
  hachikuji compile <json-dir> <out> build the spec index from spec JSON
  hachikuji complete [--pwd D] <buf> print the popup for a buffer (debug)
  hachikuji bench <file>             type each line key by key (10,000 keys); print latency
  hachikuji update                   install the newest release (uses curl)
  hachikuji version";

const ZSH: &str = include_str!("../shell/hachikuji.zsh");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["init", "zsh"] => init(),
        ["serve", dir] => {
            let _ = serve::serve(Path::new(dir));
        }
        ["compile", dir, out] => {
            if let Err(e) = compile::compile_dir(Path::new(dir), Path::new(out)) {
                eprintln!("hachikuji: {e}");
                exit(1);
            }
        }
        ["complete", "--pwd", pwd, buf] => debug(pwd, buf),
        ["complete", buf] => debug(&std::env::current_dir().unwrap().to_string_lossy(), buf),
        ["bench", file] => bench(file),
        ["update"] => update(),
        ["version"] | ["--version"] => println!("hachikuji {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("{USAGE}");
            exit(2);
        }
    }
}

/// Prints `HACHIKUJI_BIN=...; source <file>`. Uses the installed zsh file if there is one,
/// else writes the copy built into the binary to ~/.cache/hachikuji/.
fn init() {
    let exe = std::env::current_exe().unwrap_or_default();
    // No canonicalize: through Homebrew's symlinks the path stays valid after an upgrade
    // (/opt/homebrew/share/..., not /opt/homebrew/Cellar/hachikuji/<version>/...).
    let up = |n: usize| exe.ancestors().nth(n).unwrap_or(Path::new("/")).to_path_buf();
    let shipped = [up(2).join("share/hachikuji/hachikuji.zsh"), up(3).join("shell/hachikuji.zsh")];
    let file = shipped.into_iter().find(|p| p.is_file()).unwrap_or_else(|| {
        let p = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/hachikuji/hachikuji.zsh");
        if std::fs::read_to_string(&p).ok().as_deref() != Some(ZSH) {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(&p, ZSH);
        }
        p
    });
    let q = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"));
    println!("HACHIKUJI_BIN={} source {}", q(&exe), q(&file));
}

const RELEASES: &str = "https://github.com/glpecile/hachikuji/releases";

/// Installs the newest release over this one with its install.sh. The only command that uses
/// the network, through the system curl; the helper never does (plan.md, constraint 7).
fn update() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let curl = |args: &[&str]| match Command::new("curl").args(args).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => {
            eprintln!("hachikuji: cannot reach {RELEASES}");
            exit(1)
        }
    };
    let have = env!("CARGO_PKG_VERSION");
    // releases/latest redirects to releases/tag/v<version>.
    let to = curl(&["-fsS", "-o", "/dev/null", "-w", "%{redirect_url}", &format!("{RELEASES}/latest")]);
    let to = String::from_utf8_lossy(&to);
    let latest = to.rsplit_once("/tag/v").map_or("", |(_, v)| v.trim());
    match (version(latest), version(have)) {
        (Some(l), Some(h)) if l > h => {}
        (Some(_), Some(_)) => return println!("hachikuji {have} is up to date"),
        _ => {
            eprintln!("hachikuji: no release found at {RELEASES}/latest");
            exit(1)
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    if real.components().any(|c| c.as_os_str() == "Cellar") {
        return println!("hachikuji {latest} is out. Run: brew upgrade hachikuji");
    }
    if let Some(repo) = real.ancestors().nth(3).filter(|r| r.join("shell/hachikuji.zsh").is_file()) {
        return println!("hachikuji {latest} is out. Run: cd {} && git pull && make install", repo.display());
    }
    let prefix = exe.ancestors().nth(2).unwrap_or(Path::new("/"));
    println!("Updating hachikuji {have} to {latest}");
    let script = curl(&["-fsSL", &format!("{RELEASES}/download/v{latest}/install.sh")]);
    let sh = Command::new("sh").env("PREFIX", prefix).env("HACHIKUJI_UPDATE", "1").stdin(Stdio::piped()).spawn();
    let ok = sh.is_ok_and(|mut sh| sh.stdin.take().unwrap().write_all(&script).is_ok() && sh.wait().is_ok_and(|s| s.success()));
    if !ok {
        exit(1);
    }
    println!("Open a new shell to use it. Open shells keep {have} until they close.");
}

/// "1.2.3" as [1, 2, 3], so versions compare as numbers. None for anything else ("1.2.3-rc1").
fn version(v: &str) -> Option<Vec<u32>> {
    v.split('.').map(|p| p.parse().ok()).collect()
}

fn request(buf: &str, pwd: &str) -> serve::Request {
    serve::Request { seq: 1, cursor: buf.len(), pwd: pwd.into(), cols: 80, lines: 24, sel: 0, buffer: buf.into() }
}

fn debug(pwd: &str, buf: &str) {
    let e = serve::Engine { ix: serve::load_index(), cfg: render::Config::load() };
    let mut s = serve::Shared::default();
    let mut out = Vec::new();
    let r = request(buf, pwd);
    for g in serve::respond(&e, &s, &r, &mut out) {
        let items = generate::run(&g);
        s.cache.put(g.key, items);
    }
    serve::respond(&e, &s, &r, &mut out);
    let text = String::from_utf8_lossy(&out);
    let f: Vec<&str> = text.trim_end_matches('\x1e').split('\x1f').collect();
    println!("count={} replace={} insert={:?}{}", f[3], f[5], f[6], f[7]);
}

/// Latency inside the helper: parse the request frame, complete, render, write the response.
fn bench(file: &str) {
    use std::io::Write;
    let e = serve::Engine { ix: serve::load_index(), cfg: render::Config::default() };
    let mut s = serve::Shared::default();
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("hachikuji: {file}: {e}");
        exit(1)
    });
    let pwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    let mut sink = std::fs::OpenOptions::new().write(true).open("/dev/null").unwrap();
    let (mut times, mut buf, mut out) = (Vec::new(), Vec::new(), Vec::new());
    // Each line is typed one key at a time: one request per prefix, 10,000 requests in all.
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let keys = lines.iter().cycle().flat_map(|l| l.char_indices().map(|(i, c)| &l[..i + c.len_utf8()])).take(10_000);
    // Warm the generator cache first, as in a shell that has been used for a while.
    let keys: Vec<&str> = keys.collect();
    for l in &keys {
        for g in serve::respond(&e, &s, &request(l, &pwd), &mut out) {
            if !s.cache.has(&g.key) {
                let items = generate::run(&g);
                s.cache.put(g.key, items);
            }
        }
    }
    for l in keys {
        let r = request(l, &pwd);
        let raw = format!("1\0q\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0", r.seq, r.cursor, r.pwd, r.cols, r.lines, r.sel, r.buffer);
        let t = std::time::Instant::now();
        let mut rd = std::io::Cursor::new(raw.as_bytes());
        let Some(Ok(req)) = serve::parse_request(&mut rd, &mut buf) else { continue };
        serve::respond(&e, &s, &req, &mut out);
        sink.write_all(&out).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
    println!("requests={} p50={:.3}ms p99={:.3}ms max={:.3}ms", times.len(), p(0.5), p(0.99), times[times.len() - 1]);
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(version("0.10.0") > version("0.9.1"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert_eq!(version("0.1.0"), version("0.1.0"));
        assert_eq!(version("0.2.0-rc1"), None);
        assert_eq!(version(""), None);
    }
}
