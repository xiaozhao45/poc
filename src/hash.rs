use std::fmt;

use crate::err::{PocError, Res};

/// 内容寻址哈希：BLAKE3-256。
/// id = BLAKE3(类型标签 ‖ 规范载荷)（见 lib.rs TAG_*）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn compute(tag: u8, payload: &[u8]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(&[tag]);
        h.update(payload);
        let out = h.finalize();
        Hash(*out.as_bytes())
    }

    pub fn hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    pub fn short(&self) -> String {
        let h = self.hex();
        h[..8].to_string()
    }

    pub fn from_hex(s: &str) -> Res<Self> {
        let s = s.trim();
        if s.len() != 64 {
            return Err(PocError::Msg(format!(
                "哈希须为 64 位十六进制，得到 {} 位：{s}",
                s.len()
            )));
        }
        let mut b = [0u8; 32];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| PocError::Msg(format!("非法十六进制：{s}")))?;
        }
        Ok(Hash(b))
    }

    pub fn has_prefix(&self, prefix: &str) -> bool {
        self.hex().starts_with(prefix)
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}
