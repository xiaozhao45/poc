//! redb 存储：静态对象表（blobs/trees/ops）+ 指针层（composes/pool/attic/audit/meta）。
//! 每个改写命令 = 内存重建验证 + 一个写事务提交；失败无副作用。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::object::{Op, Tree};
use crate::{TAG_OP, TAG_TREE};

const T_BLOBS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blobs");
const T_TREES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("trees");
const T_OPS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("ops");
const T_COMPOSES: TableDefinition<&str, &[u8]> = TableDefinition::new("composes");
const T_POOL: TableDefinition<&[u8], &[u8]> = TableDefinition::new("pool");
const T_ATTIC: TableDefinition<&[u8], &[u8]> = TableDefinition::new("attic");
const T_AUDIT: TableDefinition<u64, &[u8]> = TableDefinition::new("audit");
const T_META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

#[derive(Debug, Clone)]
pub struct ComposeState {
    pub base: Hash,
    pub head: Hash,
    pub ops: Vec<Hash>,
}

impl ComposeState {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.base.0);
        v.extend_from_slice(&self.head.0);
        crate::object::put_u32(&mut v, self.ops.len() as u32);
        for h in &self.ops {
            v.extend_from_slice(&h.0);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Res<ComposeState> {
        let mut c = crate::object::Cursor::new(b);
        let base = c.hash()?;
        let head = c.hash()?;
        let n = c.u32()? as usize;
        let mut ops = Vec::with_capacity(n);
        for _ in 0..n {
            ops.push(c.hash()?);
        }
        c.end()?;
        Ok(ComposeState { base, head, ops })
    }
}

pub fn tree_id(t: &Tree) -> Hash {
    Hash::compute(TAG_TREE, &t.encode())
}

pub fn op_id(o: &Op) -> Hash {
    Hash::compute(TAG_OP, &o.encode())
}

pub struct Store {
    pub db: Database,
    pub root: PathBuf,
}

impl Store {
    pub fn store_path(root: &Path) -> PathBuf {
        root.join(".poc").join("store")
    }

    pub fn create(root: &Path) -> Res<Store> {
        std::fs::create_dir_all(root.join(".poc").join("swap"))?;
        let db = Database::create(Self::store_path(root))?;
        let s = Store {
            db,
            root: root.to_path_buf(),
        };
        s.init_tables()?;
        Ok(s)
    }

    pub fn open(root: &Path) -> Res<Store> {
        if !Self::store_path(root).exists() {
            return Err(PocError::NotAProject);
        }
        let db = Database::create(Self::store_path(root))?;
        Ok(Store {
            db,
            root: root.to_path_buf(),
        })
    }

    /// 自 start 向上查找 .poc/store（Project 身份 = 规范路径）。
    pub fn open_find(start: &Path) -> Res<Store> {
        let mut dir = std::fs::canonicalize(start)?;
        loop {
            if Self::store_path(&dir).exists() {
                return Self::open(&dir);
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => return Err(PocError::NotAProject),
            }
        }
    }

    fn init_tables(&self) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let _ = wtx.open_table(T_BLOBS)?;
            let _ = wtx.open_table(T_TREES)?;
            let _ = wtx.open_table(T_OPS)?;
            let _ = wtx.open_table(T_COMPOSES)?;
            let _ = wtx.open_table(T_POOL)?;
            let _ = wtx.open_table(T_ATTIC)?;
            let _ = wtx.open_table(T_AUDIT)?;
            let _ = wtx.open_table(T_META)?;
        }
        wtx.commit()?;
        Ok(())
    }

    // ---------- meta ----------

    pub fn meta_get(&self, key: &str) -> Res<Option<String>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_META)?;
        if let Some(g) = t.get(key)? {
            Ok(Some(String::from_utf8_lossy(g.value()).into_owned()))
        } else {
            Ok(None)
        }
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_META)?;
            t.insert(key, value.as_bytes())?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn meta_del(&self, key: &str) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_META)?;
            t.remove(key)?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn meta_list(&self) -> Res<Vec<(String, String)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_META)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, v) = row?;
            out.push((
                k.value().to_string(),
                String::from_utf8_lossy(v.value()).into_owned(),
            ));
        }
        Ok(out)
    }

    pub fn current_name(&self) -> Res<Option<String>> {
        self.meta_get("current")
    }

    // ---------- 对象读取 ----------

    pub fn get_blob(&self, h: &Hash) -> Res<Option<Vec<u8>>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_BLOBS)?;
        if let Some(g) = t.get(&h.0[..])? {
            Ok(Some(g.value().to_vec()))
        } else {
            Ok(None)
        }
    }

    pub fn get_tree(&self, h: &Hash) -> Res<Option<Tree>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_TREES)?;
        if let Some(g) = t.get(&h.0[..])? {
            Ok(Some(Tree::decode(g.value())?))
        } else {
            Ok(None)
        }
    }

    pub fn get_op(&self, h: &Hash) -> Res<Option<Op>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_OPS)?;
        if let Some(g) = t.get(&h.0[..])? {
            Ok(Some(Op::decode(g.value())?))
        } else {
            Ok(None)
        }
    }

    pub fn list_ops(&self) -> Res<Vec<(Hash, Op)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_OPS)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, v) = row?;
            let h = Hash(k.value().try_into().expect("键长 32"));
            out.push((h, Op::decode(v.value())?));
        }
        Ok(out)
    }

    pub fn put_tree(&self, id: &Hash, tree: &Tree) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_TREES)?;
            t.insert(&id.0[..], &tree.encode()[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn delete_op_row(&self, h: &Hash) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_OPS)?;
            t.remove(&h.0[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn delete_tree_row(&self, h: &Hash) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_TREES)?;
            t.remove(&h.0[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    // ---------- Compose ----------

    pub fn compose_get(&self, name: &str) -> Res<Option<ComposeState>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_COMPOSES)?;
        if let Some(g) = t.get(name)? {
            Ok(Some(ComposeState::decode(g.value())?))
        } else {
            Ok(None)
        }
    }

    pub fn compose_put(&self, name: &str, st: &ComposeState) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_COMPOSES)?;
            t.insert(name, &st.encode()[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn compose_delete(&self, name: &str) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_COMPOSES)?;
            t.remove(name)?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn compose_names(&self) -> Res<Vec<String>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_COMPOSES)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, _) = row?;
            out.push(k.value().to_string());
        }
        Ok(out)
    }

    // ---------- 自由池 / attic / 审计 ----------

    pub fn pool_add(&self, h: &Hash, time_ms: u64) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_POOL)?;
            let b = time_ms.to_le_bytes();
            t.insert(&h.0[..], &b[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn pool_remove(&self, h: &Hash) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_POOL)?;
            t.remove(&h.0[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn pool_list(&self) -> Res<Vec<(Hash, u64)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_POOL)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, v) = row?;
            out.push((
                Hash(k.value().try_into().expect("键长 32")),
                u64::from_le_bytes(v.value().try_into().expect("值长 8")),
            ));
        }
        Ok(out)
    }

    pub fn attic_add(&self, h: &Hash, origin: &str, time_ms: u64) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_ATTIC)?;
            let mut v = Vec::new();
            crate::object::put_str(&mut v, origin);
            crate::object::put_u64(&mut v, time_ms);
            t.insert(&h.0[..], &v[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn attic_remove(&self, h: &Hash) -> Res<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_ATTIC)?;
            t.remove(&h.0[..])?;
        }
        wtx.commit()?;
        Ok(())
    }

    pub fn attic_list(&self) -> Res<Vec<(Hash, String, u64)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_ATTIC)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, v) = row?;
            let mut c = crate::object::Cursor::new(v.value());
            let origin = c.str()?;
            let time = c.u64()?;
            out.push((Hash(k.value().try_into().expect("键长 32")), origin, time));
        }
        Ok(out)
    }

    pub fn audit_append(&self, verb: &str, ids: &[Hash], time_ms: u64) -> Res<u64> {
        let wtx = self.db.begin_write()?;
        let seq = {
            let mut m = wtx.open_table(T_META)?;
            let cur: u64 = match m.get("audit.seq")? {
                Some(g) => String::from_utf8_lossy(g.value()).parse().unwrap_or(0),
                None => 0,
            };
            let next = cur + 1;
            m.insert("audit.seq", next.to_string().as_bytes())?;
            next
        };
        {
            let mut a = wtx.open_table(T_AUDIT)?;
            let mut v = Vec::new();
            crate::object::put_str(&mut v, verb);
            crate::object::put_u32(&mut v, ids.len() as u32);
            for h in ids {
                v.extend_from_slice(&h.0);
            }
            crate::object::put_u64(&mut v, time_ms);
            a.insert(seq, &v[..])?;
        }
        wtx.commit()?;
        Ok(seq)
    }

    pub fn audit_list(&self) -> Res<Vec<(u64, String, Vec<Hash>, u64)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(T_AUDIT)?;
        let mut out = Vec::new();
        for row in t.iter()? {
            let (k, v) = row?;
            let mut c = crate::object::Cursor::new(v.value());
            let verb = c.str()?;
            let n = c.u32()? as usize;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(c.hash()?);
            }
            let time = c.u64()?;
            out.push((k.value(), verb, ids, time));
        }
        Ok(out)
    }

    // ---------- 一次提交：对象 + 指针同一写事务 ----------

    /// 记录/amend 共用。调用方已把 op_id 追加进 st.ops、更新 st.head。
    pub fn commit_op(
        &self,
        compose_name: &str,
        st: &ComposeState,
        op: &Op,
        blobs: &BTreeMap<Hash, Vec<u8>>,
        tree: &Tree,
    ) -> Res<Hash> {
        let tid = tree_id(tree);
        let oid = op_id(op);
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_BLOBS)?;
            for (h, c) in blobs {
                t.insert(&h.0[..], &c[..])?;
            }
        }
        {
            let mut t = wtx.open_table(T_TREES)?;
            t.insert(&tid.0[..], &tree.encode()[..])?;
        }
        {
            let mut t = wtx.open_table(T_OPS)?;
            t.insert(&oid.0[..], &op.encode()[..])?;
        }
        {
            let mut t = wtx.open_table(T_COMPOSES)?;
            t.insert(compose_name, &st.encode()[..])?;
        }
        wtx.commit()?;
        Ok(oid)
    }

    // ---------- gc：清除不可达对象 ----------

    pub fn gc_run(&self) -> Res<(usize, usize, usize)> {
        // 可达根：composes（base/head 树 + 栈内 op）∪ 自由池 ∪ attic
        let mut reach_ops: HashSet<Hash> = HashSet::new();
        let mut reach_trees: HashSet<Hash> = HashSet::new();

        let rtx = self.db.begin_read()?;
        {
            let ct = rtx.open_table(T_COMPOSES)?;
            for row in ct.iter()? {
                let (_, v) = row?;
                let st = ComposeState::decode(v.value())?;
                reach_trees.insert(st.base);
                reach_trees.insert(st.head);
                for h in &st.ops {
                    reach_ops.insert(*h);
                }
            }
        }
        {
            let pt = rtx.open_table(T_POOL)?;
            for row in pt.iter()? {
                let (k, _) = row?;
                reach_ops.insert(Hash(k.value().try_into().expect("键长 32")));
            }
        }
        {
            let at = rtx.open_table(T_ATTIC)?;
            for row in at.iter()? {
                let (k, _) = row?;
                reach_ops.insert(Hash(k.value().try_into().expect("键长 32")));
            }
        }
        {
            let ot = rtx.open_table(T_OPS)?;
            for h in reach_ops.clone() {
                if let Some(g) = ot.get(&h.0[..])? {
                    let op = Op::decode(g.value())?;
                    reach_trees.insert(op.pre);
                    reach_trees.insert(op.post);
                }
            }
        }
        let mut reach_blobs: HashSet<Hash> = HashSet::new();
        {
            let tt = rtx.open_table(T_TREES)?;
            for h in reach_trees.clone() {
                if let Some(g) = tt.get(&h.0[..])? {
                    for e in Tree::decode(g.value())?.entries {
                        reach_blobs.insert(e.blob);
                    }
                }
            }
        }
        drop(rtx);

        let mut del_blobs = 0usize;
        let mut del_trees = 0usize;
        let mut del_ops = 0usize;
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(T_BLOBS)?;
            let keys: Vec<Vec<u8>> = t
                .iter()?
                .filter_map(|r| r.ok().map(|(k, _)| k.value().to_vec()))
                .collect();
            for k in keys {
                let h = Hash(k[..].try_into().unwrap());
                if !reach_blobs.contains(&h) {
                    t.remove(&k[..])?;
                    del_blobs += 1;
                }
            }
        }
        {
            let mut t = wtx.open_table(T_TREES)?;
            let keys: Vec<Vec<u8>> = t
                .iter()?
                .filter_map(|r| r.ok().map(|(k, _)| k.value().to_vec()))
                .collect();
            for k in keys {
                let h = Hash(k[..].try_into().unwrap());
                if !reach_trees.contains(&h) {
                    t.remove(&k[..])?;
                    del_trees += 1;
                }
            }
        }
        {
            let mut t = wtx.open_table(T_OPS)?;
            let keys: Vec<Vec<u8>> = t
                .iter()?
                .filter_map(|r| r.ok().map(|(k, _)| k.value().to_vec()))
                .collect();
            for k in keys {
                let h = Hash(k[..].try_into().unwrap());
                if !reach_ops.contains(&h) {
                    t.remove(&k[..])?;
                    del_ops += 1;
                }
            }
        }
        wtx.commit()?;
        Ok((del_blobs, del_trees, del_ops))
    }
}
