//! 不可变对象：tree（快照）与 op（Operation）及其规范编码（可哈希、长度前缀）。

use std::collections::BTreeMap;

use crate::err::{PocError, Res};
use crate::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Regular = 0,
    Exec = 1,
    Symlink = 2,
}

impl Mode {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(x: u8) -> Res<Self> {
        match x {
            0 => Ok(Mode::Regular),
            1 => Ok(Mode::Exec),
            2 => Ok(Mode::Symlink),
            _ => Err(PocError::Msg(format!("非法 mode 字节：{x}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub mode: Mode,
    pub blob: Hash,
}

/// 快照：路径按字节序的扁平表（不建子树对象，见 §3.2）。
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        put_u32(&mut v, self.entries.len() as u32);
        for e in &self.entries {
            put_str(&mut v, &e.path);
            v.push(e.mode.as_u8());
            v.extend_from_slice(&e.blob.0);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Res<Tree> {
        let mut c = Cursor::new(b);
        let n = c.u32()?;
        let mut entries = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let path = c.str()?;
            let mode = Mode::from_u8(c.byte()?)?;
            let blob = c.hash()?;
            entries.push(TreeEntry { path, mode, blob });
        }
        c.end()?;
        Ok(Tree { entries })
    }

    pub fn to_map(&self) -> BTreeMap<String, (Mode, Hash)> {
        self.entries
            .iter()
            .map(|e| (e.path.clone(), (e.mode, e.blob)))
            .collect()
    }

    pub fn from_map(map: &BTreeMap<String, (Mode, Hash)>) -> Tree {
        let mut entries: Vec<TreeEntry> = map
            .iter()
            .map(|(p, (m, h))| TreeEntry {
                path: p.clone(),
                mode: *m,
                blob: *h,
            })
            .collect();
        entries.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
        Tree { entries }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Author {
    pub name: String,
    pub email: String,
}

impl Author {
    pub fn display(&self) -> String {
        if self.email.is_empty() {
            self.name.clone()
        } else {
            format!("{} <{}>", self.name, self.email)
        }
    }
}

/// Operation：自包含的（前态, 后态）快照对 + 作者 + 消息 + 时间。
/// 不绑定父节点（重排改变前态），id = BLAKE3(TAG_OP ‖ 规范编码)，author/msg/time 进入身份。
#[derive(Debug, Clone)]
pub struct Op {
    pub pre: Hash,
    pub post: Hash,
    pub author: Author,
    pub msg: String,
    pub time_ms: u64,
}

impl Op {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.pre.0);
        v.extend_from_slice(&self.post.0);
        put_str(&mut v, &self.author.name);
        put_str(&mut v, &self.author.email);
        put_str(&mut v, &self.msg);
        put_u64(&mut v, self.time_ms);
        v
    }

    pub fn decode(b: &[u8]) -> Res<Op> {
        let mut c = Cursor::new(b);
        let pre = c.hash()?;
        let post = c.hash()?;
        let name = c.str()?;
        let email = c.str()?;
        let msg = c.str()?;
        let time_ms = c.u64()?;
        c.end()?;
        Ok(Op {
            pre,
            post,
            author: Author { name, email },
            msg,
            time_ms,
        })
    }
}

// ---------- 规范编码原语 ----------

pub fn put_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

pub fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

pub fn put_u64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

pub fn put_str(v: &mut Vec<u8>, s: &str) {
    put_u16(v, s.len() as u16);
    v.extend_from_slice(s.as_bytes());
}

pub struct Cursor<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cursor { b, o: 0 }
    }

    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if self.o + n > self.b.len() {
            return Err(PocError::Msg("对象编码越界".into()));
        }
        let s = &self.b[self.o..self.o + n];
        self.o += n;
        Ok(s)
    }

    pub fn byte(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Res<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    pub fn u32(&mut self) -> Res<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes(s.try_into().unwrap()))
    }

    pub fn u64(&mut self) -> Res<u64> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes(s.try_into().unwrap()))
    }

    pub fn hash(&mut self) -> Res<Hash> {
        let s = self.take(32)?;
        Ok(Hash(s.try_into().unwrap()))
    }

    pub fn str(&mut self) -> Res<String> {
        let n = self.u16()? as usize;
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| PocError::Msg("对象编码含非法 UTF-8".into()))
    }

    pub fn end(&mut self) -> Res<()> {
        if self.o != self.b.len() {
            return Err(PocError::Msg("对象编码有多余字节".into()));
        }
        Ok(())
    }
}
