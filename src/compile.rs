//! Turns Fig-style spec JSON (functions already removed by the build step) into the index.
//! Each `<name>.json` under the input folder is one root spec; `aws/s3.json` is root "aws/s3".

use crate::index::{builtin, flag, kind, Node, Writer};
use serde_json::Value;
use std::io;
use std::path::Path;

pub fn compile_dir(dir: &Path, out: &Path) -> io::Result<()> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut w = Writer::default();
    for (name, path) in &files {
        let text = std::fs::read_to_string(path)?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
        add_root(&mut w, name, &v);
    }
    std::fs::write(out, w.finish())
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) -> io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect(base, &p, out)?;
        } else if p.extension().is_some_and(|x| x == "json") {
            let rel = p.strip_prefix(base).unwrap().with_extension("");
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
    Ok(())
}

pub fn add_root(w: &mut Writer, name: &str, v: &Value) {
    let n = cmd(w, v);
    let id = w.seq(vec![n]);
    w.root(name, id);
}

fn names(v: &Value) -> Vec<&str> {
    match v.get("name") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect(),
        _ => vec![],
    }
}

fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn truthy(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Null) | None => false,
        Some(_) => true, // a number (isRepeatable: 2) or a string (requiresSeparator: ":")
    }
}

/// Fig allows one object or an array for args, generators and templates.
fn many(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Null) | None => vec![],
        Some(x) => vec![x],
    }
}

fn base(w: &mut Writer, v: &Value, k: u32) -> Node {
    let mut ns: Vec<String> = names(v).into_iter().map(String::from).collect();
    let ins = str_field(v, "insertValue");
    if !ins.is_empty() {
        ns.push(format!("\x01{ins}"));
    }
    let ns: Vec<&str> = ns.iter().map(|x| x.as_str()).collect();
    let mut n = Node {
        kind: k,
        names: w.strs(&ns),
        // A hidden node is never shown; its names are still needed to parse what was typed.
        desc: if truthy(v, "hidden") { 0 } else { w.s(&short(str_field(v, "description"))) },
        priority: v.get("priority").and_then(|p| p.as_f64()).unwrap_or(50.0) as i32,
        ..Node::default()
    };
    if truthy(v, "hidden") {
        n.flags |= flag::HIDDEN;
    }
    n
}

/// The popup shows at most 60 columns of a description: keep the first line, 50 characters (the index must stay under 15 MB).
fn short(d: &str) -> String {
    let l = d.lines().next().unwrap_or("").trim();
    match l.char_indices().nth(50) {
        Some((i, _)) => format!("{}…", &l[..i]),
        None => l.to_string(),
    }
}

/// Builds the children first (bottom-up), then stores them as one contiguous list.
fn children(w: &mut Writer, items: Vec<(&Value, u32)>) -> (u32, u32) {
    let nodes: Vec<Node> = items
        .iter()
        .map(|(v, k)| match *k {
            kind::CMD => cmd(w, v),
            kind::OPT => opt(w, v),
            kind::ARG => arg(w, v),
            kind::SUG => sug(w, v),
            _ => generator(w, v),
        })
        .collect();
    let n = nodes.len() as u32;
    (if n == 0 { 0 } else { w.seq(nodes) }, n)
}

fn cmd(w: &mut Writer, v: &Value) -> Node {
    let mut n = base(w, v, kind::CMD);
    n.extra = w.s(str_field(v, "loadSpec"));
    let mut items: Vec<(&Value, u32)> = Vec::new();
    items.extend(many(v.get("subcommands")).into_iter().map(|x| (x, kind::CMD)));
    items.extend(many(v.get("options")).into_iter().map(|x| (x, kind::OPT)));
    items.extend(many(v.get("args")).into_iter().map(|x| (x, kind::ARG)));
    (n.child, n.nchild) = children(w, items);
    n
}

fn opt(w: &mut Writer, v: &Value) -> Node {
    let mut n = base(w, v, kind::OPT);
    let args = many(v.get("args"));
    for (f, k) in [(flag::REPEAT, "isRepeatable"), (flag::PERSIST, "isPersistent"), (flag::REQ_SEP, "requiresSeparator"), (flag::REQ_SEP, "requiresEquals")] {
        if truthy(v, k) {
            n.flags |= f;
        }
    }
    if truthy(v, "requiresArgument") || args.first().is_some_and(|a| !truthy(a, "isOptional")) {
        n.flags |= flag::REQ_ARG;
    }
    let excl: Vec<&str> = many(v.get("exclusiveOn")).iter().filter_map(|x| x.as_str()).collect();
    n.extra = w.strs(&excl);
    (n.child, n.nchild) = children(w, args.into_iter().map(|x| (x, kind::ARG)).collect());
    n
}

fn template_flags(t: Option<&Value>) -> u32 {
    many(t).iter().fold(0, |f, x| match x.as_str() {
        Some("filepaths") => f | flag::T_FILES,
        Some("folders") => f | flag::T_FOLDERS,
        _ => f,
    })
}

fn arg(w: &mut Writer, v: &Value) -> Node {
    // The popup never shows an arg's own name or description, only its suggestions: skip
    // them (229,000 args in the Fig specs).
    let mut n = Node { kind: kind::ARG, priority: 50, ..Node::default() };
    for (f, k) in [(flag::VARIADIC, "isVariadic"), (flag::OPTIONAL, "isOptional"), (flag::IS_CMD, "isCommand")] {
        if truthy(v, k) {
            n.flags |= f;
        }
    }
    n.extra = w.s(str_field(v, "loadSpec"));
    n.flags |= template_flags(v.get("template"));
    let gens = many(v.get("generators"));
    let mut items: Vec<(&Value, u32)> = many(v.get("suggestions")).into_iter().map(|x| (x, kind::SUG)).collect();
    for g in gens {
        n.flags |= template_flags(g.get("template"));
        if g.get("script").is_some() || g.get("builtin").is_some() {
            items.push((g, kind::GEN));
        }
    }
    (n.child, n.nchild) = children(w, items);
    n
}

fn sug(w: &mut Writer, v: &Value) -> Node {
    if let Some(s) = v.as_str() {
        return Node { kind: kind::SUG, names: w.strs(&[s]), priority: 50, ..Node::default() };
    }
    let mut n = base(w, v, kind::SUG);
    match str_field(v, "type") {
        "file" => n.flags |= flag::SUG_FILE,
        "folder" => n.flags |= flag::SUG_FOLDER,
        _ => {}
    }
    n
}

fn generator(w: &mut Writer, v: &Value) -> Node {
    let mut n = Node { kind: kind::GEN, priority: 50, ..Node::default() };
    n.flags |= builtin::by_name(str_field(v, "builtin")) << flag::BUILTIN_SHIFT;
    match v.get("script") {
        Some(Value::String(s)) => {
            n.flags |= flag::SHELL;
            n.names = w.strs(&[s]);
        }
        Some(Value::Array(a)) => {
            let argv: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
            n.names = w.strs(&argv);
        }
        _ => {}
    }
    n.extra = w.s(v.get("splitOn").and_then(|x| x.as_str()).unwrap_or("\n"));
    n
}
