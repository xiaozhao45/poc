//! graft：唯一的重写原语（DESIGN §4.2）。
//! merge3(base=X, ours=Z, theirs=Y) → W：树级按路径三路裁决，文本行级三路合并
//! （`similar::merge`，隔离在本模块，可替换）。
//! 冲突分两类：文本/删除修改/mode 冲突 → 可经步骤协议解决（带标记全文交交换文件）；
//! 二进制冲突 → 不可步骤化，直接拒绝。

use std::collections::{BTreeMap, BTreeSet};

use similar::merge::TextMerge;

use crate::TAG_BLOB;
use crate::db::{Store, tree_id};
use crate::diff::is_binary;
use crate::err::PocError;
use crate::hash::Hash;
use crate::object::{Mode, Tree, TreeEntry};

/// 树级三路合并的产出：新快照 + 合并产生的**新 blob**（调用方须随同一事务入库）。
pub struct Merged {
    pub tree: Tree,
    pub tree_id: Hash,
    pub blobs: BTreeMap<Hash, Vec<u8>>,
}

impl Merged {}

/// 单文件冲突：带冲突标记的全文（ours/theirs 段落），供交换文件与解决回填。
#[derive(Debug, Clone)]
pub struct FileConflict {
    pub path: String,
    pub marked: Vec<u8>,
}

/// 合并失败：可步骤化的冲突 + 不可步骤化的二进制冲突。
#[derive(Debug, Clone, Default)]
pub struct MergeFail {
    pub conflicts: Vec<FileConflict>,
    pub binaries: Vec<String>,
}

impl MergeFail {
    pub fn is_empty(&self) -> bool {
        self.conflicts.is_empty() && self.binaries.is_empty()
    }

    pub fn file_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .conflicts
            .iter()
            .map(|c| c.path.clone())
            .chain(self.binaries.iter().cloned())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// 冲突解决回调：返回 Some(内容) = 用家已定内容；None = 未解决。
pub type ResolveFn<'a> = dyn FnMut(&str, &FileConflict) -> Option<Vec<u8>> + 'a;

/// 行级文本三路合并：无冲突 → 合并字节；有冲突 → Err。
fn merge_text(base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<Vec<u8>, ()> {
    let m = TextMerge::from_lines(base, ours, theirs);
    if m.is_conflicted() {
        return Err(());
    }
    let mut out = Vec::new();
    m.to_writer(&mut out).map_err(|_| ())?;
    Ok(out)
}

/// 带标记全文（未解决区域以 <<<<<<< / ======= / >>>>>>> 包裹）。
fn marked_text(base: &[u8], ours: &[u8], theirs: &[u8]) -> Vec<u8> {
    let m = TextMerge::from_lines(base, ours, theirs);
    let mut out = Vec::new();
    let _ = m.to_writer(&mut out);
    out
}

fn wrap_markers(ours: &[u8], theirs: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"<<<<<<< ours\n");
    v.extend_from_slice(ours);
    if !ours.ends_with(b"\n") {
        v.push(b'\n');
    }
    v.extend_from_slice(b"=======\n");
    v.extend_from_slice(theirs);
    if !theirs.ends_with(b"\n") {
        v.push(b'\n');
    }
    v.extend_from_slice(b">>>>>>> theirs\n");
    v
}

/// mode 三方裁决：单方改 → 取改方；双方同改 → 任一；三方各异 → 冲突。
fn resolve_mode(b: Mode, o: Mode, t: Mode) -> Option<Mode> {
    if o == b {
        Some(t)
    } else if t == b || o == t {
        Some(o)
    } else {
        None
    }
}

fn blob_bytes(store: &Store, h: &Hash) -> Vec<u8> {
    store.get_blob(h).ok().flatten().unwrap_or_default()
}

/// merge3(base, ours, theirs) → W。冲突经 resolve 回调定夺；未解决的冲突（或二进制
/// 冲突）→ Err(MergeFail)，无副作用。
pub fn merge_trees_resolving(
    store: &Store,
    base: &Tree,
    ours: &Tree,
    theirs: &Tree,
    resolve: &mut ResolveFn,
) -> Result<Merged, MergeFail> {
    let bm = base.to_map();
    let om = ours.to_map();
    let tm = theirs.to_map();
    let mut paths: BTreeSet<&String> = BTreeSet::new();
    paths.extend(bm.keys());
    paths.extend(om.keys());
    paths.extend(tm.keys());

    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut blobs: BTreeMap<Hash, Vec<u8>> = BTreeMap::new();
    let mut fail = MergeFail::default();

    for p in paths {
        let b = bm.get(p.as_str());
        let o = om.get(p.as_str());
        let t = tm.get(p.as_str());
        // 单方改动 / 双方一致：直接取。外层 None = 需深度合并；内层 None = 路径删除。
        let easy: Option<Option<(Mode, Hash)>> = if o == t {
            Some(o.copied())
        } else if b == o {
            Some(t.copied())
        } else if b == t {
            Some(o.copied())
        } else {
            None
        };
        match easy {
            Some(Some((mode, blob))) => entries.push(TreeEntry {
                path: p.clone(),
                mode,
                blob,
            }),
            Some(None) => {} // 双方一致地删除（或删除方为唯一改动方）
            None => {
                // 双方都改了
                match (b, o, t) {
                    (Some(&(bm_, bh)), Some(&(om_, oh)), Some(&(tm_, th))) => {
                        // 内容
                        let content: Option<Vec<u8>> = if oh == th {
                            Some(blob_bytes(store, &oh))
                        } else {
                            let bb = blob_bytes(store, &bh);
                            let ob = blob_bytes(store, &oh);
                            let tb = blob_bytes(store, &th);
                            if is_binary(&bb) || is_binary(&ob) || is_binary(&tb) {
                                fail.binaries.push(p.clone());
                                continue;
                            }
                            merge_text(&bb, &ob, &tb).ok()
                        };
                        // mode
                        let mode = resolve_mode(bm_, om_, tm_);
                        match (content, mode) {
                            (Some(c), Some(m)) => {
                                let h = Hash::compute(TAG_BLOB, &c);
                                blobs.insert(h, c);
                                entries.push(TreeEntry {
                                    path: p.clone(),
                                    mode: m,
                                    blob: h,
                                });
                            }
                            (content, mode) => {
                                // 冲突：文本冲突给带标记全文；仅 mode 冲突给一致内容包标记
                                let marked = match content {
                                    Some(c) => wrap_markers(&c, &c),
                                    None => {
                                        let bb = blob_bytes(store, &bh);
                                        let ob = blob_bytes(store, &oh);
                                        let tb = blob_bytes(store, &th);
                                        marked_text(&bb, &ob, &tb)
                                    }
                                };
                                let fc = FileConflict {
                                    path: p.clone(),
                                    marked,
                                };
                                match resolve(p, &fc) {
                                    Some(bytes) => {
                                        let h = Hash::compute(TAG_BLOB, &bytes);
                                        blobs.insert(h, bytes);
                                        entries.push(TreeEntry {
                                            path: p.clone(),
                                            mode: mode.unwrap_or(om_),
                                            blob: h,
                                        });
                                    }
                                    None => fail.conflicts.push(fc),
                                }
                            }
                        }
                    }
                    _ => {
                        // 一侧删除、另一侧修改
                        let ob = match o {
                            Some((_, h)) => blob_bytes(store, h),
                            None => Vec::new(),
                        };
                        let tb = match t {
                            Some((_, h)) => blob_bytes(store, h),
                            None => Vec::new(),
                        };
                        let marked = wrap_markers(&ob, &tb);
                        let fc = FileConflict {
                            path: p.clone(),
                            marked,
                        };
                        match resolve(p, &fc) {
                            Some(bytes) => {
                                // 空内容 = 用户裁定删除
                                if !bytes.is_empty() {
                                    let h = Hash::compute(TAG_BLOB, &bytes);
                                    blobs.insert(h, bytes);
                                    entries.push(TreeEntry {
                                        path: p.clone(),
                                        mode: Mode::Regular,
                                        blob: h,
                                    });
                                }
                            }
                            None => fail.conflicts.push(fc),
                        }
                    }
                }
            }
        }
    }
    if !fail.is_empty() {
        return Err(fail);
    }
    entries.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let tree = Tree { entries };
    Ok(Merged {
        tree_id: tree_id(&tree),
        tree,
        blobs,
    })
}

fn none_resolve(_p: &str, _fc: &FileConflict) -> Option<Vec<u8>> {
    None
}

/// merge3 严格形态：任何冲突 → Err(文件列表)。
pub fn merge_trees(
    store: &Store,
    base: &Tree,
    ours: &Tree,
    theirs: &Tree,
) -> Result<Merged, Vec<String>> {
    merge_trees_resolving(store, base, ours, theirs, &mut none_resolve).map_err(|f| f.file_names())
}

/// graft((X→Y), Z) → W：把"X→Y 的变更"施加到快照 Z 上（§4.2）。
pub fn graft(store: &Store, x: &Tree, y: &Tree, z: &Tree) -> Result<Merged, Vec<String>> {
    merge_trees(store, x, z, y)
}

/// graft 的可解决形态（步骤协议回放用）。
pub fn graft_resolving<'a>(
    store: &Store,
    x: &Tree,
    y: &Tree,
    z: &Tree,
    resolve: &mut ResolveFn<'a>,
) -> Result<Merged, MergeFail> {
    merge_trees_resolving(store, x, z, y, resolve)
}

/// 合并失败 → PocError（附文件列表与后续指引）。
pub fn fail_err(fail: &MergeFail) -> PocError {
    use rust_i18n::t;
    if !fail.binaries.is_empty() {
        return PocError::Conflict(
            t!("merge.binary_conflict", paths = fail.binaries.join(", ")).to_string(),
        );
    }
    PocError::Conflict(
        t!(
            "merge.conflict_summary",
            paths = fail.file_names().join(", ")
        )
        .to_string(),
    )
}
