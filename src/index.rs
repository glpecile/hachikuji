//! The spec index: one file, read with mmap, zero copy. All shells share its pages.
//!
//! Layout (all numbers are little-endian u32):
//!   header   magic "HKJ1", then n_str n_list n_node n_root, then the offsets of the 6 sections
//!   stroff   n_str + 1 offsets into blob          string i = blob[stroff[i]..stroff[i+1]]
//!   blob     UTF-8 bytes
//!   listoff  n_list + 1 offsets into list data    list i = lists[listoff[i]..listoff[i+1]] (string ids)
//!   lists    string ids
//!   nodes    n_node records of 6 words: kind<<28|flags, names, desc, child,
//!            nchild|priority<<16 (u16, i16), extra
//! A node's insertValue is the last entry of its names list, marked with a leading 0x01.
//!   roots    n_root pairs (name string id, node id), sorted by name bytes
//! String 0 is "" and list 0 is empty, so 0 means "none".

use std::collections::HashMap;
use std::io;
use std::ops::Range;

pub const MAGIC: &[u8; 4] = b"HKJ1";
const HEADER: usize = 4 + 10 * 4;
const NODE_WORDS: usize = 6;

pub mod kind {
    pub const CMD: u32 = 1;
    pub const OPT: u32 = 2;
    pub const ARG: u32 = 3;
    pub const SUG: u32 = 4;
    pub const GEN: u32 = 5;
}

pub mod flag {
    pub const HIDDEN: u32 = 1 << 0;
    pub const REQ_ARG: u32 = 1 << 1;
    pub const REPEAT: u32 = 1 << 2;
    pub const PERSIST: u32 = 1 << 3;
    pub const VARIADIC: u32 = 1 << 4;
    pub const OPTIONAL: u32 = 1 << 5;
    pub const REQ_SEP: u32 = 1 << 6;
    pub const IS_CMD: u32 = 1 << 7;
    pub const T_FILES: u32 = 1 << 8;
    pub const T_FOLDERS: u32 = 1 << 9;
    /// Generator: the script is one string for /bin/sh -c (else an argv list).
    pub const SHELL: u32 = 1 << 10;
    pub const SUG_FILE: u32 = 1 << 11;
    pub const SUG_FOLDER: u32 = 1 << 12;
    /// Bits 16..24: built-in generator id (see `builtin`).
    pub const BUILTIN_SHIFT: u32 = 16;
}

pub mod builtin {
    pub const GIT_BRANCHES: u32 = 1;
    pub const GIT_REMOTES: u32 = 2;
    pub const GIT_TAGS: u32 = 3;
    pub const EXECUTABLES: u32 = 4;
    pub fn by_name(s: &str) -> u32 {
        match s {
            "git-branches" => GIT_BRANCHES,
            "git-remotes" => GIT_REMOTES,
            "git-tags" => GIT_TAGS,
            "executables" => EXECUTABLES,
            _ => 0,
        }
    }
}

/// One node: a command, option, arg, suggestion or generator.
/// `extra` is: OPT exclusiveOn list, GEN splitOn string, CMD/ARG loadSpec string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Node {
    pub kind: u32,
    pub flags: u32,
    pub names: u32,
    pub desc: u32,
    pub child: u32,
    pub nchild: u32,
    pub priority: i32,
    pub extra: u32,
}

impl Node {
    pub fn has(&self, f: u32) -> bool {
        self.flags & f != 0
    }
    pub fn builtin(&self) -> u32 {
        (self.flags >> flag::BUILTIN_SHIFT) & 0xff
    }
}

enum Data {
    Map(memmap2::Mmap),
    Owned(Vec<u8>),
}

pub struct Index {
    data: Data,
    n_str: usize,
    n_list: usize,
    n_node: usize,
    n_root: usize,
    o: [usize; 6], // stroff blob listoff lists nodes roots
}

impl Index {
    pub fn open(path: &std::path::Path) -> io::Result<Index> {
        let f = std::fs::File::open(path)?;
        // Safety: the index is written once by the compiler and never changed in place.
        let m = unsafe { memmap2::Mmap::map(&f)? };
        Index::from_data(Data::Map(m))
    }

    pub fn from_bytes(b: Vec<u8>) -> io::Result<Index> {
        Index::from_data(Data::Owned(b))
    }

    pub fn empty() -> Index {
        Index::from_bytes(Writer::default().finish()).expect("empty index")
    }

    fn from_data(data: Data) -> io::Result<Index> {
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "bad index");
        let b: &[u8] = match &data {
            Data::Map(m) => m,
            Data::Owned(v) => v,
        };
        if b.len() < HEADER || &b[..4] != MAGIC {
            return Err(bad());
        }
        let w = |i: usize| u32::from_le_bytes(b[4 + 4 * i..8 + 4 * i].try_into().unwrap()) as usize;
        let (n_str, n_list, n_node, n_root) = (w(0), w(1), w(2), w(3));
        let o = [w(4), w(5), w(6), w(7), w(8), w(9)];
        // Check that every section fits, so later reads cannot go out of bounds.
        let ends = [
            o[0] + 4 * (n_str + 1),
            o[2] + 4 * (n_list + 1),
            o[4] + 4 * NODE_WORDS * n_node,
            o[5] + 8 * n_root,
        ];
        if ends.iter().any(|&e| e > b.len()) {
            return Err(bad());
        }
        let ix = Index { data, n_str, n_list, n_node, n_root, o };
        let blob_len = ix.u(ix.o[0] + 4 * n_str) as usize;
        let list_len = ix.u(ix.o[2] + 4 * n_list) as usize;
        if ix.o[1] + blob_len > ix.bytes().len() || ix.o[3] + 4 * list_len > ix.bytes().len() {
            return Err(bad());
        }
        Ok(ix)
    }

    fn bytes(&self) -> &[u8] {
        match &self.data {
            Data::Map(m) => m,
            Data::Owned(v) => v,
        }
    }

    fn u(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.bytes()[off..off + 4].try_into().unwrap())
    }

    pub fn str(&self, id: u32) -> &str {
        let id = id as usize;
        if id >= self.n_str {
            return "";
        }
        let a = self.u(self.o[0] + 4 * id) as usize;
        let z = self.u(self.o[0] + 4 * id + 4) as usize;
        std::str::from_utf8(self.bytes().get(self.o[1] + a..self.o[1] + z).unwrap_or(&[])).unwrap_or("")
    }

    pub fn list(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let id = id as usize;
        let r: Range<usize> = if id >= self.n_list {
            0..0
        } else {
            self.u(self.o[2] + 4 * id) as usize..self.u(self.o[2] + 4 * id + 4) as usize
        };
        r.map(move |i| self.u(self.o[3] + 4 * i))
    }

    pub fn strs(&self, list: u32) -> impl Iterator<Item = &str> + '_ {
        self.list(list).map(move |s| self.str(s))
    }

    pub fn node(&self, id: u32) -> Node {
        if id as usize >= self.n_node {
            return Node::default();
        }
        let b = self.o[4] + 4 * NODE_WORDS * id as usize;
        let w = |i: usize| self.u(b + 4 * i);
        Node {
            kind: w(0) >> 28,
            flags: w(0) & 0x0fff_ffff,
            names: w(1),
            desc: w(2),
            child: w(3),
            nchild: w(4) & 0xffff,
            priority: (w(4) >> 16) as u16 as i16 as i32,
            extra: w(5),
        }
    }

    pub fn children(&self, n: &Node) -> impl Iterator<Item = (u32, Node)> + '_ {
        (n.child..n.child.saturating_add(n.nchild)).map(move |i| (i, self.node(i)))
    }

    pub fn names(&self, n: &Node) -> impl Iterator<Item = &str> + '_ {
        self.strs(n.names).filter(|s| !s.starts_with('\x01'))
    }

    /// The spec's insertValue, or "".
    pub fn insert_value(&self, n: &Node) -> &str {
        self.strs(n.names).find_map(|s| s.strip_prefix('\x01')).unwrap_or("")
    }

    pub fn name(&self, n: &Node) -> &str {
        self.names(n).next().unwrap_or("")
    }

    fn root_at(&self, i: usize) -> (&str, u32) {
        let b = self.o[5] + 8 * i;
        (self.str(self.u(b)), self.u(b + 4))
    }

    pub fn root(&self, name: &str) -> Option<u32> {
        let (mut lo, mut hi) = (0, self.n_root);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let (k, v) = self.root_at(mid);
            match k.as_bytes().cmp(name.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(v),
            }
        }
        None
    }

    pub fn roots(&self) -> impl Iterator<Item = (&str, u32)> + '_ {
        (0..self.n_root).map(move |i| self.root_at(i))
    }
}

/// Builds an index in memory. Strings and lists are de-duplicated.
#[derive(Default)]
pub struct Writer {
    strs: Vec<String>,
    str_ids: HashMap<String, u32>,
    lists: Vec<Vec<u32>>,
    list_ids: HashMap<Vec<u32>, u32>,
    nodes: Vec<Node>,
    node_seqs: HashMap<Vec<Node>, u32>,
    roots: Vec<(String, u32)>,
}

impl Writer {
    pub fn s(&mut self, s: &str) -> u32 {
        if self.strs.is_empty() {
            self.strs.push(String::new());
            self.str_ids.insert(String::new(), 0);
        }
        if let Some(&i) = self.str_ids.get(s) {
            return i;
        }
        let i = self.strs.len() as u32;
        self.strs.push(s.to_string());
        self.str_ids.insert(s.to_string(), i);
        i
    }

    pub fn l(&mut self, items: Vec<u32>) -> u32 {
        if self.lists.is_empty() {
            self.lists.push(Vec::new());
            self.list_ids.insert(Vec::new(), 0);
        }
        if let Some(&i) = self.list_ids.get(&items) {
            return i;
        }
        let i = self.lists.len() as u32;
        self.lists.push(items.clone());
        self.list_ids.insert(items, i);
        i
    }

    pub fn strs(&mut self, items: &[&str]) -> u32 {
        let ids = items.iter().map(|s| self.s(s)).collect();
        self.l(ids)
    }

    /// Stores sibling nodes contiguously (children are a range). Identical lists are stored
    /// once: specs repeat the same options in many subcommands (aws: 37 MB of JSON).
    /// Build bottom-up: each node's own children must already be stored.
    pub fn seq(&mut self, nodes: Vec<Node>) -> u32 {
        if let Some(&s) = self.node_seqs.get(&nodes) {
            return s;
        }
        let start = self.nodes.len() as u32;
        self.nodes.extend_from_slice(&nodes);
        self.node_seqs.insert(nodes, start);
        start
    }

    pub fn root(&mut self, name: &str, node: u32) {
        self.roots.push((name.to_string(), node));
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.s("");
        self.l(Vec::new());
        self.roots.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        self.roots.dedup_by(|a, b| a.0 == b.0);
        let roots: Vec<(u32, u32)> = std::mem::take(&mut self.roots)
            .into_iter()
            .map(|(n, id)| (self.s(&n), id))
            .collect();

        let mut stroff = Vec::with_capacity(self.strs.len() + 1);
        let mut blob = Vec::new();
        for s in &self.strs {
            stroff.push(blob.len() as u32);
            blob.extend_from_slice(s.as_bytes());
        }
        stroff.push(blob.len() as u32);
        let mut listoff = Vec::with_capacity(self.lists.len() + 1);
        let mut lists = Vec::new();
        for l in &self.lists {
            listoff.push(lists.len() as u32);
            lists.extend_from_slice(l);
        }
        listoff.push(lists.len() as u32);

        let mut out = Vec::new();
        let words = |v: &[u32], out: &mut Vec<u8>| v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
        out.extend_from_slice(MAGIC);
        out.resize(HEADER, 0);
        let mut o = [0u32; 6];
        o[0] = out.len() as u32;
        words(&stroff, &mut out);
        o[1] = out.len() as u32;
        out.extend_from_slice(&blob);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        o[2] = out.len() as u32;
        words(&listoff, &mut out);
        o[3] = out.len() as u32;
        words(&lists, &mut out);
        o[4] = out.len() as u32;
        for n in &self.nodes {
            // kind and flags share a word; so do the child count (u16) and the priority (i16).
            let pr = n.priority.clamp(i16::MIN as i32, i16::MAX as i32) as i16 as u16 as u32;
            let r = [n.kind << 28 | n.flags, n.names, n.desc, n.child, n.nchild.min(0xffff) | pr << 16, n.extra];
            words(&r, &mut out);
        }
        o[5] = out.len() as u32;
        for (k, v) in &roots {
            words(&[*k, *v], &mut out);
        }
        let head = [
            self.strs.len() as u32,
            self.lists.len() as u32,
            self.nodes.len() as u32,
            roots.len() as u32,
            o[0],
            o[1],
            o[2],
            o[3],
            o[4],
            o[5],
        ];
        for (i, x) in head.iter().enumerate() {
            out[4 + 4 * i..8 + 4 * i].copy_from_slice(&x.to_le_bytes());
        }
        out
    }
}
