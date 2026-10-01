//! 差分层：tree 级路径差分 + 行级 unified diff / 统计（similar）。
//! M2 的 graft（三路合并）也将落在本模块旁的 merge.rs。

use std::collections::BTreeMap;

use crate::hash::Hash;
use crate::object::Mode;

pub struct TreeDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
}

impl TreeDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

pub fn diff_maps(
    old: &BTreeMap<String, (Mode, Hash)>,
    new: &BTreeMap<String, (Mode, Hash)>,
) -> TreeDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    for (p, nv) in new {
        match old.get(p) {
            None => added.push(p.clone()),
            Some(ov) if ov != nv => modified.push(p.clone()),
            _ => {}
        }
    }
    for p in old.keys() {
        if !new.contains_key(p) {
            removed.push(p.clone());
        }
    }
    TreeDiff {
        added,
        removed,
        modified,
    }
}

pub fn is_binary(b: &[u8]) -> bool {
    b.iter().take(8000).any(|&x| x == 0)
}

pub fn hunk_count(old: &[u8], new: &[u8]) -> usize {
    if is_binary(old) || is_binary(new) {
        return 1;
    }
    let o = String::from_utf8_lossy(old);
    let n = String::from_utf8_lossy(new);
    similar::TextDiff::from_lines(o.as_ref(), n.as_ref())
        .grouped_ops(3)
        .len()
}

pub fn unified(old: &[u8], new: &[u8], a_path: &str, b_path: &str) -> String {
    if is_binary(old) || is_binary(new) {
        return "（二进制文件：按整体替换处理，不产生行级差异）\n".to_string();
    }
    let o = String::from_utf8_lossy(old);
    let n = String::from_utf8_lossy(new);
    similar::TextDiff::from_lines(o.as_ref(), n.as_ref())
        .unified_diff()
        .context_radius(3)
        .header(a_path, b_path)
        .to_string()
}

/// （新增行数, 删除行数）
pub fn stat_lines(old: &[u8], new: &[u8]) -> (usize, usize) {
    if is_binary(old) || is_binary(new) {
        return (0, 0);
    }
    let o = String::from_utf8_lossy(old);
    let n = String::from_utf8_lossy(new);
    let d = similar::TextDiff::from_lines(o.as_ref(), n.as_ref());
    let mut add = 0usize;
    let mut del = 0usize;
    for change in d.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => add += 1,
            similar::ChangeTag::Delete => del += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (add, del)
}
