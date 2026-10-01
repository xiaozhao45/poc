//! 工作区扫描（.pocignore 语义经 ignore crate）、快照构建、物化（checkout 用）。

use std::collections::BTreeMap;
use std::path::Path;

use crate::db::Store;
use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::object::{Mode, Tree};
use crate::TAG_BLOB;

#[derive(Debug, Clone)]
pub struct WorkItem {
    pub mode: Mode,
    pub data: Vec<u8>,
}

pub type WorkMap = BTreeMap<String, WorkItem>;

fn rel_to_string(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let mut s = String::new();
    for c in rel.components() {
        if !s.is_empty() {
            s.push('/');
        }
        s.push_str(&c.as_os_str().to_string_lossy());
    }
    s
}

fn target_to_bytes(t: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        t.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        t.to_string_lossy().into_owned().into_bytes()
    }
}

fn exec_bit(p: &Path) -> Mode {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(p) {
            Ok(m) if m.permissions().mode() & 0o111 != 0 => Mode::Exec,
            _ => Mode::Regular,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        Mode::Regular
    }
}

/// 扫描工作区：跳过 .poc/ 与 .pocignore 规则命中的路径；空目录不追踪。
pub fn scan(root: &Path) -> Res<WorkMap> {
    let mut out = WorkMap::new();
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(true).git_ignore(false).require_git(false);
    builder.add_custom_ignore_filename(".pocignore");
    let walker = builder
        .filter_entry(|e| e.file_name().to_string_lossy() != ".poc")
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| PocError::Msg(format!("扫描失败：{e}")))?;
        if entry.depth() == 0 {
            continue;
        }
        let ft = match entry.file_type() {
            Some(t) => t,
            None => continue,
        };
        let path = entry.path().to_path_buf();
        let rel = rel_to_string(root, &path);
        if ft.is_symlink() {
            let target = std::fs::read_link(&path)?;
            out.insert(
                rel,
                WorkItem {
                    mode: Mode::Symlink,
                    data: target_to_bytes(&target),
                },
            );
        } else if ft.is_file() {
            let data = std::fs::read(&path)?;
            let mode = exec_bit(&path);
            out.insert(rel, WorkItem { mode, data });
        }
    }
    Ok(out)
}

pub fn hash_of(item: &WorkItem) -> Hash {
    Hash::compute(TAG_BLOB, &item.data)
}

pub fn tree_of(map: &WorkMap) -> Tree {
    let m: BTreeMap<String, (Mode, Hash)> = map
        .iter()
        .map(|(p, w)| (p.clone(), (w.mode, hash_of(w))))
        .collect();
    Tree::from_map(&m)
}

pub fn blob_map(map: &WorkMap) -> BTreeMap<Hash, Vec<u8>> {
    map.iter()
        .map(|(_, w)| (hash_of(w), w.data.clone()))
        .collect()
}

/// 磁盘内容（mode, hash）与 head 树是否完全一致 —— 工作区干净判定。
pub fn disk_matches_tree(map: &WorkMap, tree: &Tree) -> bool {
    let disk: BTreeMap<String, (Mode, Hash)> = map
        .iter()
        .map(|(p, w)| (p.clone(), (w.mode, hash_of(w))))
        .collect();
    disk == tree.to_map()
}

/// 把 tree 物化到工作区：删除多余文件、写入全部条目（切换 Compose 用）。
pub fn materialize(root: &Path, tree: &Tree, store: &Store, current: &WorkMap) -> Res<()> {
    let want = tree.to_map();
    for p in current.keys() {
        if !want.contains_key(p) {
            let full = root.join(p);
            if full.symlink_metadata().is_ok() {
                let _ = std::fs::remove_file(&full);
            }
        }
    }
    for (rel, (mode, bh)) in &want {
        let data = store
            .get_blob(bh)?
            .ok_or_else(|| PocError::Msg(format!("对象缺失：blob {}", bh.short())))?;
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut write = true;
        if *mode == Mode::Symlink {
            #[cfg(unix)]
            {
                if let Ok(cur) = std::fs::read_link(&full) {
                    if cur.as_os_str().to_string_lossy() == String::from_utf8_lossy(&data) {
                        write = false;
                    }
                }
            }
        } else if let Ok(cur) = std::fs::read(&full) {
            if cur == data {
                write = false;
            }
        }
        if !write {
            continue;
        }
        match mode {
            Mode::Symlink => {
                let _ = std::fs::remove_file(&full);
                #[cfg(unix)]
                {
                    let target = String::from_utf8_lossy(&data).into_owned();
                    std::os::unix::fs::symlink(target, &full)?;
                }
                #[cfg(not(unix))]
                {
                    std::fs::write(&full, &data)?;
                }
            }
            _ => {
                std::fs::write(&full, &data)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perm = if *mode == Mode::Exec { 0o755 } else { 0o644 };
                    std::fs::set_permissions(&full, std::fs::Permissions::from_mode(perm))?;
                }
            }
        }
    }
    Ok(())
}
