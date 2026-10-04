//! 步骤协议（DESIGN §1.6 / §4.4）：交互点解构为非交互步骤。
//! 冲突 → 每冲突文件一份带标记的交换文件（.poc/swap/step/）+ meta 步骤记录；
//! `poc --commit` 以交换文件内容回放原操作（graft 链以交换文件内容为准）；
//! `poc --cancel` 放弃。步骤原子：提交前重建只在内存进行，工作区保持原样。

use std::path::PathBuf;

use crate::db::Store;
use crate::err::{PocError, Res};
use crate::object::{put_str, put_u64, Cursor};

pub const STEP_META: &str = "step";

pub const KIND_LIFT: u8 = 1;
pub const KIND_POP: u8 = 2;
pub const KIND_COMPACT: u8 = 3;
pub const KIND_MERGE: u8 = 4;

pub fn kind_label(kind: u8) -> String {
    use rust_i18n::t;
    match kind {
        KIND_LIFT => t!("step.kind_lift").to_string(),
        KIND_POP => t!("step.kind_pop").to_string(),
        KIND_COMPACT => t!("step.kind_compact").to_string(),
        KIND_MERGE => t!("step.kind_merge").to_string(),
        _ => "??".into(),
    }
}

/// 一条待办步骤。guards = （compose, head 十六进制）指纹：提交时前置状态必须未变。
/// swaps = （merge 调用序号, 冲突路径, 交换文件名）。
#[derive(Debug, Clone)]
pub struct Step {
    pub kind: u8,
    pub ids: Vec<String>,
    pub message: String,
    pub result_name: String,
    pub guards: Vec<(String, String)>,
    pub swaps: Vec<(u64, String, String)>,
}

impl Step {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(self.kind);
        crate::object::put_u32(&mut v, self.ids.len() as u32);
        for s in &self.ids {
            put_str(&mut v, s);
        }
        put_str(&mut v, &self.message);
        put_str(&mut v, &self.result_name);
        crate::object::put_u32(&mut v, self.guards.len() as u32);
        for (n, h) in &self.guards {
            put_str(&mut v, n);
            put_str(&mut v, h);
        }
        crate::object::put_u32(&mut v, self.swaps.len() as u32);
        for (seq, path, fname) in &self.swaps {
            put_u64(&mut v, *seq);
            put_str(&mut v, path);
            put_str(&mut v, fname);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Res<Step> {
        let mut c = Cursor::new(b);
        let kind = c.byte()?;
        let n = c.u32()? as usize;
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(c.str()?);
        }
        let message = c.str()?;
        let result_name = c.str()?;
        let g = c.u32()? as usize;
        let mut guards = Vec::with_capacity(g);
        for _ in 0..g {
            let name = c.str()?;
            let head = c.str()?;
            guards.push((name, head));
        }
        let s = c.u32()? as usize;
        let mut swaps = Vec::with_capacity(s);
        for _ in 0..s {
            let seq = c.u64()?;
            let path = c.str()?;
            let fname = c.str()?;
            swaps.push((seq, path, fname));
        }
        c.end()?;
        Ok(Step {
            kind,
            ids,
            message,
            result_name,
            guards,
            swaps,
        })
    }
}

pub fn swap_root(root: &std::path::Path) -> PathBuf {
    root.join(".poc").join("swap").join("step")
}

pub fn sanitize_path(p: &str) -> String {
    p.replace('/', "__")
}

/// 读取待办步骤（无 → None）。
pub fn load(store: &Store) -> Res<Option<Step>> {
    match store.meta_get_bytes(STEP_META)? {
        Some(b) => Ok(Some(Step::decode(&b)?)),
        None => Ok(None),
    }
}

/// 建步骤：先写交换文件，meta 记录最后落（崩溃安全：meta 在 = 步骤完整）。
pub fn create(
    store: &Store,
    root: &std::path::Path,
    step: &Step,
    contents: &[(u64, String, Vec<u8>)],
) -> Res<()> {
    let dir = swap_root(root);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    for (seq, path, bytes) in contents {
        let fname = format!("{seq:04}__{}", sanitize_path(path));
        std::fs::write(dir.join(&fname), bytes)?;
    }
    store.meta_set_bytes(STEP_META, &step.encode())?;
    Ok(())
}

/// 放弃步骤：删交换目录 + meta。
pub fn clear(store: &Store, root: &std::path::Path) -> Res<()> {
    let _ = std::fs::remove_dir_all(swap_root(root));
    store.meta_del(STEP_META)
}

/// 交换文件是否仍含未解决的冲突标记。
pub fn has_markers(b: &[u8]) -> bool {
    b.split(|c| *c == b'\n').any(|l| {
        l.starts_with(b"<<<<<<< ") || l.starts_with(b">>>>>>> ") || l == b"======="
    })
}

/// 待办步骤的存在性门禁（新交互命令遇待办步骤即拒绝）。
pub fn gate(store: &Store) -> Res<()> {
    use rust_i18n::t;
    if load(store)?.is_some() {
        return Err(PocError::Msg(t!("step.err_pending").to_string()));
    }
    Ok(())
}
