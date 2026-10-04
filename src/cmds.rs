//! `poc` 主工具的命令实现（安全本地操作；销毁族在 dcmds，链接在 cpoc——均不在此）。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use globset::Glob;
use rust_i18n::t;

use anstream::println;

use crate::config;
use crate::db::{ComposeState, Store, tree_id};
use crate::diff::{self, diff_maps};
use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::log::{
    EVT_AMEND, EVT_CMP_NEW, EVT_COMPACT, EVT_INIT, EVT_MERGE, EVT_POP, EVT_PUSH, Event, POOL_SCOPE,
};
use crate::merge;
use crate::object::{Mode, Op, Tree};
use crate::theme::{Token, paint};

/// 冲突解决回调：（merge 调用序号, 冲突路径, 带标记全文）→ 用家已定内容。
/// 回放（`poc --commit`）时由交换文件供给；普通执行恒为 None。
type Resolver<'a> = dyn Fn(usize, &str, &merge::FileConflict) -> Option<Vec<u8>> + 'a;
use crate::render;
use crate::tree::{self, WorkItem, WorkMap};
use crate::ui;
use crate::{TAG_BLOB, TAG_OP};

pub struct Ctx {
    pub globals: crate::cli::Globals,
    pub store: Store,
    pub root: PathBuf,
}

impl Ctx {
    pub fn open(globals: crate::cli::Globals) -> Res<Self> {
        let store = Store::open_find(Path::new("."))?;
        let root = store.root.clone();
        Ok(Ctx {
            globals,
            store,
            root,
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn require_current(ctx: &Ctx) -> Res<(String, ComposeState)> {
    let name = ctx
        .store
        .current_name()?
        .ok_or_else(|| PocError::Msg(t!("error.no_current_create").to_string()))?;
    let st = ctx
        .store
        .compose_get(&name)?
        .ok_or_else(|| PocError::Msg(t!("error.compose_not_found", name = name).to_string()))?;
    Ok((name, st))
}

// ---------- proj ----------

pub fn cmd_proj(new_name: Option<String>, path: Option<String>) -> Res<()> {
    let target: PathBuf = match (new_name, path) {
        (Some(_), Some(_)) => return Err(PocError::Usage(t!("proj.err_both").to_string())),
        (Some(name), None) => {
            if name.is_empty() || name.contains('/') {
                return Err(PocError::Usage(t!("proj.err_bad_name").to_string()));
            }
            let p = std::env::current_dir()?.join(&name);
            if p.exists() {
                return Err(PocError::Msg(
                    t!("proj.err_dir_exists", path = p.display().to_string()).to_string(),
                ));
            }
            std::fs::create_dir_all(&p)?;
            p
        }
        (None, Some(p)) => {
            let p = PathBuf::from(p);
            std::fs::create_dir_all(&p)?;
            std::fs::canonicalize(&p)?
        }
        (None, None) => std::env::current_dir()?,
    };
    let target = std::fs::canonicalize(&target)?;
    init_at(&target)?;
    println!(
        "{}",
        t!("proj.initialized", path = target.display().to_string())
    );
    Ok(())
}

pub fn init_at(root: &Path) -> Res<Store> {
    if Store::store_path(root).exists() {
        return Err(PocError::Msg(
            t!("proj.err_already", path = root.display().to_string()).to_string(),
        ));
    }
    let store = Store::create(root)?;
    let empty = Tree::default();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    store.init_project(
        &name,
        &empty,
        &Event::new(
            EVT_INIT,
            "main",
            None,
            "",
            "proj initialized",
            "base = empty snapshot",
            now_ms(),
        ),
    )?;
    Ok(store)
}

// ---------- opt ----------

pub struct OptArgs {
    pub ids: Vec<String>,
    pub message: Option<String>,
    pub include: Option<String>,
    pub free: bool,
    pub compact: bool,
    pub amend: bool,
}

pub fn cmd_opt(ctx: &Ctx, a: OptArgs) -> Res<()> {
    crate::step::gate(&ctx.store)?;
    if a.amend {
        if !a.ids.is_empty() {
            return Err(PocError::Usage(t!("error.amend_no_ids").to_string()));
        }
        if a.free || a.compact {
            return Err(PocError::Usage(t!("error.amend_flags").to_string()));
        }
        return cmd_opt_amend(ctx, a);
    }
    if a.free && a.compact {
        return Err(PocError::Usage(t!("error.free_compact").to_string()));
    }
    if !a.ids.is_empty() {
        if a.compact {
            return opt_compact(ctx, a, None);
        }
        if a.free {
            return opt_pop(ctx, a, None);
        }
        return opt_lift(ctx, a, None);
    }
    if a.free || a.compact {
        return Err(PocError::Usage(t!("error.ids_required").to_string()));
    }
    cmd_opt_create(ctx, a)
}

/// 冲突收尾：二进制冲突直接拒绝；其余建待办步骤（交换文件 + meta）。
/// 输出 = 共享的步骤节渲染器；错误经 wrap 打 `error:` 前缀行。
#[allow(clippy::too_many_arguments)] // 参数天然并列（种类/ids/守卫/待办/二进制），拆 struct 反而失真
fn finish_conflict(
    ctx: &Ctx,
    kind: u8,
    ids: &[String],
    message: &str,
    result_name: &str,
    guards: &[(&str, &Hash)],
    pending: &[(u64, String, Vec<u8>)],
    binaries: &[String],
) -> Res<()> {
    if !binaries.is_empty() {
        return Err(merge::fail_err(&merge::MergeFail {
            conflicts: Vec::new(),
            binaries: binaries.to_vec(),
        }));
    }
    let step = crate::step::Step {
        kind,
        ids: ids.to_vec(),
        message: message.to_string(),
        result_name: result_name.to_string(),
        guards: guards
            .iter()
            .map(|(n, h)| (n.to_string(), h.hex()))
            .collect(),
        swaps: pending
            .iter()
            .map(|(seq, path, _)| {
                (
                    *seq,
                    path.clone(),
                    format!("{seq:04}__{}", crate::step::sanitize_path(path)),
                )
            })
            .collect(),
    };
    crate::step::create(&ctx.store, &ctx.root, &step, pending)?;
    // 非 -s 且 TTY：打开编辑器逐个处理交换文件（§1.6）
    if !ctx.globals.step && crate::ui::stdout_is_tty() {
        for (seq, path, _) in pending {
            let fname = format!("{seq:04}__{}", crate::step::sanitize_path(path));
            let _ = crate::ui::open_editor(&crate::step::swap_root(&ctx.root).join(fname));
        }
    }
    anstream::print!("{}", render::step_section(&step, &ctx.root));
    Err(PocError::Msg(t!("step.conflict_created").to_string()))
}

/// -i 限定：入选路径以工作区为准（含删除 = 不放入）；未入选路径保留 head 内容。
fn build_candidate(
    ctx: &Ctx,
    head_tree: &Tree,
    include: Option<&str>,
) -> Res<(WorkMap, Tree, BTreeMap<Hash, Vec<u8>>)> {
    let work = tree::scan(&ctx.root)?;
    let candidate: WorkMap = match include {
        None => work,
        Some(pat) => {
            let glob = Glob::new(pat)
                .map_err(|e| PocError::Usage(t!("error.bad_glob", e = e.to_string()).to_string()))?
                .compile_matcher();
            let mut m: WorkMap = WorkMap::new();
            for (p, w) in &work {
                if glob.is_match(p) {
                    m.insert(p.clone(), w.clone());
                }
            }
            for (p, (mode, bh)) in &head_tree.to_map() {
                if !glob.is_match(p)
                    && let Some(b) = ctx.store.get_blob(bh)?
                {
                    m.insert(
                        p.clone(),
                        WorkItem {
                            mode: *mode,
                            data: b,
                        },
                    );
                }
            }
            m
        }
    };
    let blobs = tree::blob_map(&candidate);
    let tree = tree::tree_of(&candidate);
    Ok((candidate, tree, blobs))
}

fn take_message(ctx: &Ctx, given: Option<String>) -> Res<String> {
    if let Some(m) = given {
        let m = m.trim().to_string();
        if m.is_empty() {
            return Err(PocError::Usage(t!("error.empty_message").to_string()));
        }
        return Ok(m);
    }
    if ctx.globals.step {
        return Err(PocError::Msg(t!("error.step_no_editor").to_string()));
    }
    let swap = ctx.root.join(".poc").join("swap");
    std::fs::create_dir_all(&swap)?;
    let f = swap.join(format!("msg-{}", std::process::id()));
    std::fs::write(&f, "")?;
    let r = ui::open_editor(&f).and_then(|()| std::fs::read_to_string(&f).map_err(PocError::Io));
    let _ = std::fs::remove_file(&f);
    let content = r?;
    let m = content.trim().to_string();
    if m.is_empty() {
        return Err(PocError::Msg(t!("error.message_cancelled").to_string()));
    }
    Ok(m)
}

fn print_record_summary(
    ctx: &Ctx,
    head_map: &BTreeMap<String, (Mode, Hash)>,
    d: &diff::TreeDiff,
    candidate: &WorkMap,
) -> Res<()> {
    let _ = (ctx, head_map, candidate); // hunk 计数已从前端移除；保留签名以便后续扩展
    println!(
        "{}",
        render::counts_summary(d.added.len(), d.removed.len(), d.modified.len())
    );
    Ok(())
}

/// `--verbose` 的文件清单行（+/−/M 着色）。
fn print_file_list(d: &diff::TreeDiff) {
    for p in &d.added {
        println!("{}", paint(Token::Added, format!("+ {p}")));
    }
    for p in &d.removed {
        println!("{}", paint(Token::Removed, format!("- {p}")));
    }
    for p in &d.modified {
        println!("{}", paint(Token::Updated, format!("M {p}")));
    }
}

pub(crate) fn cmd_opt_create(ctx: &Ctx, a: OptArgs) -> Res<()> {
    let (compose_name, mut st) = require_current(ctx)?;
    let head_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    let head_map = head_tree.to_map();
    let (candidate, tree, blobs) = build_candidate(ctx, &head_tree, a.include.as_deref())?;
    let d = diff_maps(&head_map, &tree.to_map());
    if d.is_empty() {
        return Err(PocError::Msg(t!("error.no_changes").to_string()));
    }
    let (author, _) = config::resolve_identity(&ctx.store)?;
    let author_name = author.name.clone();
    let msg = take_message(ctx, a.message)?;
    let op = Op {
        pre: st.head,
        post: tree_id(&tree),
        author,
        msg,
        time_ms: now_ms(),
    };
    let op_id = Hash::compute(TAG_OP, &op.encode());
    if ctx.globals.dry_run {
        if !ctx.globals.quiet {
            print_record_summary(ctx, &head_map, &d, &candidate)?;
        }
        println!(
            "{}",
            t!(
                "dryrun.record",
                id = paint(Token::Id, op_id.short()),
                msg = op.msg,
                name = compose_name
            )
        );
        return Ok(());
    }
    st.ops.push(op_id);
    st.origs.push(op_id);
    st.head = op.post;
    let ev = Event::new(
        EVT_PUSH,
        &compose_name,
        Some(op_id),
        &author_name,
        &op.msg,
        "",
        op.time_ms,
    );
    ctx.store
        .commit_op(&compose_name, &st, &op, &blobs, &tree, Some(&ev))?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, op_id.short()));
        return Ok(());
    }
    print_record_summary(ctx, &head_map, &d, &candidate)?;
    println!(
        "{}",
        t!(
            "record.created",
            id = paint(Token::Id, op_id.short()),
            msg = op.msg
        )
    );
    if ctx.globals.verbose {
        print_file_list(&d);
    }
    Ok(())
}

pub(crate) fn cmd_opt_amend(ctx: &Ctx, a: OptArgs) -> Res<()> {
    let (compose_name, mut st) = require_current(ctx)?;
    let top_id = *st
        .ops
        .last()
        .ok_or_else(|| PocError::Msg(t!("error.stack_empty_amend").to_string()))?;
    let top = ctx
        .store
        .get_op(&top_id)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
    let head_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    let (_candidate, tree, blobs) = build_candidate(ctx, &head_tree, a.include.as_deref())?;
    let (author, _) = config::resolve_identity(&ctx.store)?;
    let author_name = author.name.clone();
    let msg = take_message(ctx, a.message)?;
    let op = Op {
        pre: top.pre,
        post: tree_id(&tree),
        author,
        msg,
        time_ms: now_ms(),
    };
    let op_id = Hash::compute(TAG_OP, &op.encode());
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.amend",
                name = compose_name,
                id = paint(Token::Id, op_id.short()),
                msg = op.msg
            )
        );
        return Ok(());
    }
    let old_head = st.head;
    st.ops.pop();
    st.ops.push(op_id);
    // 槽位的用户 id 同步改绑到新 op：否则旧 id（已入池）仍占据 pos，
    // 新 id 反而无法被 opt <id> 命中（QA 发现，2026-10-02）
    st.origs.pop();
    st.origs.push(op_id);
    st.head = op.post;
    let ev = Event::new(
        EVT_AMEND,
        &compose_name,
        Some(op_id),
        &author_name,
        &op.msg,
        &format!("original {} pooled", top_id.short()),
        op.time_ms,
    );
    ctx.store
        .amend_top(&compose_name, &st, &op, &blobs, &tree, &top_id, &ev)?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, op_id.short()));
        return Ok(());
    }
    println!(
        "{}",
        t!(
            "amend.rewrote",
            name = compose_name,
            id = paint(Token::Id, op_id.short()),
            msg = op.msg
        )
    );
    println!("{}", render::head_line(&old_head, &st.head));
    println!("{}", render::pooled_line(&[top_id]));
    Ok(())
}

/// 工作区干净检查（相对给定 head）；通过则返回工作区快照供后续物化使用。
fn require_clean(ctx: &Ctx, head: &Hash) -> Res<WorkMap> {
    let head_tree = ctx
        .store
        .get_tree(head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    let work = tree::scan(&ctx.root)?;
    if !tree::disk_matches_tree(&work, &head_tree) {
        return Err(PocError::Dirty(t!("error.dirty_record").to_string()));
    }
    Ok(work)
}

/// 栈编辑会话：内存重建，任一步冲突即整体丢弃（§3.4，无副作用）。
/// 置换产生的共轭是新对象；被替换下来的原对象一律入池（非破坏原则，§0 目标 7）。
struct StackEdit {
    base: Hash,
    head: Hash,
    ops: Vec<(Hash, Op)>, // 当前共轭形态（自底向上），.0 = 该形态的对象 id
    orig: Vec<Hash>,      // 每槽对应用户视角的原始 id
    pos: HashMap<Hash, usize>,
    new_ops: Vec<(Hash, Op)>,
    new_trees: BTreeMap<Hash, Tree>,
    new_blobs: BTreeMap<Hash, Vec<u8>>,
    pool_adds: Vec<Hash>,
    seq: usize,                           // merge 调用计数（步骤交换文件标识）
    pending: Vec<(u64, String, Vec<u8>)>, // 未解决冲突（seq, 路径, 带标记全文）
    binaries: Vec<String>,                // 二进制冲突（不可步骤化）
}

impl StackEdit {
    fn load(ctx: &Ctx, st: &ComposeState) -> Res<Self> {
        let mut ops = Vec::with_capacity(st.ops.len());
        let mut orig = Vec::with_capacity(st.ops.len());
        let mut pos = HashMap::new();
        if st.origs.len() != st.ops.len() {
            return Err(PocError::Msg(t!("error.data_origs").to_string()));
        }
        for (i, h) in st.ops.iter().enumerate() {
            let op = ctx
                .store
                .get_op(h)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
            ops.push((*h, op));
            let o = st.origs[i];
            orig.push(o);
            pos.insert(o, i);
        }
        Ok(StackEdit {
            base: st.base,
            head: st.head,
            ops,
            orig,
            pos,
            new_ops: Vec::new(),
            new_trees: BTreeMap::new(),
            new_blobs: BTreeMap::new(),
            pool_adds: Vec::new(),
            seq: 0,
            pending: Vec::new(),
            binaries: Vec::new(),
        })
    }

    fn record_fail(&mut self, seq: u64, fail: &merge::MergeFail) {
        for fc in &fail.conflicts {
            self.pending.push((seq, fc.path.clone(), fc.marked.clone()));
        }
        self.binaries.extend(fail.binaries.iter().cloned());
    }

    fn pool_add(&mut self, h: Hash) {
        if !self.pool_adds.contains(&h) {
            self.pool_adds.push(h);
        }
    }

    /// 把用户给出的 id 规约为槽位的原始 id：命中 orig 直接用；
    /// 命中某槽位的当前形态 id（共轭/amend 后的 id）也归到该槽位。
    fn canon(&self, h: &Hash) -> Option<Hash> {
        if self.pos.contains_key(h) {
            return Some(*h);
        }
        self.ops
            .iter()
            .position(|(cid, _)| cid == h)
            .map(|i| self.orig[i])
    }

    fn head_tree(&self, ctx: &Ctx) -> Res<Tree> {
        let h = self.ops.last().map(|(_, o)| o.post).unwrap_or(self.base);
        self.tree(ctx, &h)
    }

    /// 取树：先查本次编辑新铸的树，再查 store（置换链上的中间树尚未入库）。
    fn tree(&self, ctx: &Ctx, h: &Hash) -> Res<Tree> {
        if let Some(t) = self.new_trees.get(h) {
            return Ok(t.clone());
        }
        ctx.store
            .get_tree(h)?
            .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))
    }

    /// 相邻置换 (a=ops[p], b=ops[p+1]) → (b', a')：总值不变，head 不变（§2 代数依据）。
    /// 返回 false = 置换冲突（已记入 pending/binaries，调用方应停止并转步骤协议）。
    fn swap_up(&mut self, ctx: &Ctx, p: usize, res: Option<&Resolver<'_>>) -> Res<bool> {
        let (aid, a) = self.ops[p].clone();
        let (bid, b) = self.ops[p + 1].clone();
        let base_t = self.tree(ctx, &b.pre)?;
        let ours_t = self.tree(ctx, &a.pre)?;
        let theirs_t = self.tree(ctx, &b.post)?;
        let my_seq = self.seq;
        let mut sub = |path: &str, fc: &merge::FileConflict| -> Option<Vec<u8>> {
            match res {
                Some(f) => f(my_seq, path, fc),
                None => None,
            }
        };
        let m = match merge::graft_resolving(&ctx.store, &base_t, &theirs_t, &ours_t, &mut sub) {
            Ok(m) => m,
            Err(fail) => {
                self.record_fail(my_seq as u64, &fail);
                return Ok(false);
            }
        };
        self.seq += 1;
        self.new_blobs.extend(m.blobs);
        self.new_trees.insert(m.tree_id, m.tree);
        let bp = Op {
            pre: a.pre,
            post: m.tree_id,
            author: b.author,
            msg: b.msg,
            time_ms: b.time_ms,
        };
        let ap = Op {
            pre: m.tree_id,
            post: b.post,
            author: a.author,
            msg: a.msg,
            time_ms: a.time_ms,
        };
        let bpid = Hash::compute(TAG_OP, &bp.encode());
        let apid = Hash::compute(TAG_OP, &ap.encode());
        self.new_ops.push((bpid, bp.clone()));
        self.new_ops.push((apid, ap.clone()));
        let oa = self.orig[p];
        let ob = self.orig[p + 1];
        self.orig[p] = ob;
        self.orig[p + 1] = oa;
        self.pos.insert(ob, p);
        self.pos.insert(oa, p + 1);
        self.pool_add(aid);
        self.pool_add(bid);
        self.ops[p] = (bpid, bp);
        self.ops[p + 1] = (apid, ap);
        Ok(true)
    }

    /// 把原始 id 对应的元素逐格升到栈顶（弹出/聚拢共用）。false = 冲突已记录。
    fn lift_to_top(&mut self, ctx: &Ctx, orig: &Hash, res: Option<&Resolver<'_>>) -> Res<bool> {
        let mut p = *self
            .pos
            .get(orig)
            .ok_or_else(|| PocError::Msg(t!("error.lift_missing").to_string()))?;
        while p + 1 < self.ops.len() {
            if !self.swap_up(ctx, p, res)? {
                return Ok(false);
            }
            p += 1;
        }
        Ok(true)
    }

    /// 弹出栈顶元素入池（orig 应已在栈顶），head 回退重算。
    fn pop_top(&mut self, orig: &Hash) {
        let p = self.ops.len() - 1;
        let (cid, _) = self.ops.remove(p);
        self.orig.remove(p);
        self.pos.remove(orig);
        self.pool_add(cid);
        self.head = self.ops.last().map(|(_, o)| o.post).unwrap_or(self.base);
    }

    /// 他栈/池内 op 铸共轭压顶（入栈原语；head 延长）。原对象原地保留。
    /// false = 嫁接冲突（已记入 pending/binaries）。
    fn push_conjugate(&mut self, ctx: &Ctx, src: &Op, res: Option<&Resolver<'_>>) -> Res<bool> {
        let base_t = self.tree(ctx, &src.pre)?;
        let ours_t = self.head_tree(ctx)?;
        let theirs_t = self.tree(ctx, &src.post)?;
        let my_seq = self.seq;
        let mut sub = |path: &str, fc: &merge::FileConflict| -> Option<Vec<u8>> {
            match res {
                Some(f) => f(my_seq, path, fc),
                None => None,
            }
        };
        let m = match merge::graft_resolving(&ctx.store, &base_t, &theirs_t, &ours_t, &mut sub) {
            Ok(m) => m,
            Err(fail) => {
                self.record_fail(my_seq as u64, &fail);
                return Ok(false);
            }
        };
        self.seq += 1;
        if m.tree_id == self.head {
            return Err(PocError::Msg(t!("error.lift_noop").into()));
        }
        self.new_blobs.extend(m.blobs);
        self.new_trees.insert(m.tree_id, m.tree);
        let conj = Op {
            pre: self.head,
            post: m.tree_id,
            author: src.author.clone(),
            msg: src.msg.clone(),
            time_ms: src.time_ms,
        };
        let cid = Hash::compute(TAG_OP, &conj.encode());
        self.new_ops.push((cid, conj.clone()));
        self.head = m.tree_id;
        self.pos.insert(cid, self.ops.len());
        self.orig.push(cid);
        self.ops.push((cid, conj));
        Ok(true)
    }

    /// 把 sel（栈序的原始 id）聚拢为相邻块（sel[0] 位置不动）。false = 冲突已记录。
    fn gather(&mut self, ctx: &Ctx, sel: &[Hash], res: Option<&Resolver<'_>>) -> Res<bool> {
        let mut anchor = sel[0];
        for &s in &sel[1..] {
            let target = self.pos[&anchor] + 1;
            let mut p = self.pos[&s];
            while p > target {
                // 下移一格：与上一槽位互换
                if !self.swap_up(ctx, p - 1, res)? {
                    return Ok(false);
                }
                p -= 1;
            }
            while p < target {
                // 上移一格
                if !self.swap_up(ctx, p, res)? {
                    return Ok(false);
                }
                p += 1;
            }
            anchor = s;
        }
        Ok(true)
    }

    /// 把 [q, q+k) 的块原位替换为复合 op（head 不变），块成员入池。
    fn seal_block(&mut self, q: usize, k: usize, cid: Hash, c: Op) {
        let mut block = Vec::new();
        for i in q..q + k {
            block.push(self.ops[i].0);
        }
        for b in block {
            self.pool_add(b);
        }
        let sel0 = self.orig[q];
        for i in q..q + k {
            self.pos.remove(&self.orig[i]);
        }
        self.new_ops.push((cid, c.clone()));
        self.ops.splice(q..q + k, [(cid, c)]);
        self.orig[q] = sel0;
        self.orig.drain(q + 1..q + k);
        self.pos.insert(sel0, q);
    }

    fn state(&self) -> ComposeState {
        ComposeState {
            base: self.base,
            head: self.head,
            ops: self.ops.iter().map(|(h, _)| *h).collect(),
            origs: self.orig.clone(),
        }
    }
}

/// `opt <ids>`：提升重排（显式入栈原语）。栈内者逐格置换升顶——head 不变（P1）；
/// 池内/他栈者铸共轭压顶——head 延长。被替换的原对象一律入自由池或原地保留。
/// res = Some 时冲突经交换文件内容解决（步骤回放）；None 时冲突转步骤协议。
pub(crate) fn opt_lift(ctx: &Ctx, a: OptArgs, res: Option<&Resolver<'_>>) -> Res<()> {
    if a.include.is_some() {
        return Err(PocError::Usage(t!("error.lift_no_include").to_string()));
    }
    if a.message.is_some() {
        return Err(PocError::Usage(t!("error.lift_no_message").to_string()));
    }
    let (compose_name, st0) = require_current(ctx)?;
    let mut resolved: Vec<(Hash, Op)> = Vec::new();
    for s in &a.ids {
        resolved.push(resolve_op(ctx, s)?);
    }
    let work = require_clean(ctx, &st0.head)?;
    let mut ed = StackEdit::load(ctx, &st0)?;
    let head_before = ed.head;
    let mut from_stack = 0usize;
    let mut from_elsewhere = 0usize;
    for (h, op) in &resolved {
        let slot = ed.canon(h);
        let ok = if let Some(o) = slot {
            ed.lift_to_top(ctx, &o, res)?
        } else {
            ed.push_conjugate(ctx, op, res)?
        };
        if !ok {
            break;
        }
        if slot.is_some() {
            from_stack += 1;
        } else {
            from_elsewhere += 1;
        }
    }
    if !ed.pending.is_empty() || !ed.binaries.is_empty() {
        return finish_conflict(
            ctx,
            crate::step::KIND_LIFT,
            &a.ids,
            "",
            "",
            &[(compose_name.as_str(), &st0.head)],
            &ed.pending,
            &ed.binaries,
        );
    }
    let head_after = ed.head;
    let shorts: Vec<String> = resolved.iter().map(|(h, _)| h.short()).collect();
    let conj_list: Vec<String> = ed.new_ops.iter().map(|(h, _)| h.short()).collect();
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.lift",
                ids = shorts.join(" "),
                n = ed.new_ops.len(),
                from = paint(Token::Id, head_before.short()),
                to = paint(Token::Id, head_after.short())
            )
        );
        return Ok(());
    }
    let ev = Event::new(
        EVT_PUSH,
        &compose_name,
        Some(resolved.last().unwrap().0),
        "",
        &format!("lifted {} to top", shorts.join(" ")),
        &format!(
            "stack reorder {}, applied {}; head -> {}",
            from_stack,
            from_elsewhere,
            head_after.short()
        ),
        now_ms(),
    );
    ctx.store.commit_rewrite(
        &compose_name,
        &ed.state(),
        &ed.new_ops,
        &ed.new_trees,
        &ed.new_blobs,
        &ed.pool_adds,
        &ev,
    )?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, shorts.join(" ")));
        return Ok(());
    }
    if head_after != head_before {
        let t = ctx
            .store
            .get_tree(&head_after)?
            .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
        tree::materialize(&ctx.root, &t, &ctx.store, &work)?;
    }
    let result_line = if from_elsewhere == 0 {
        t!("stack.lifted", ids = shorts.join(" "), name = compose_name)
    } else {
        t!("stack.applied", ids = shorts.join(" "), name = compose_name)
    };
    println!("{}", result_line);
    println!("{}", render::head_line(&head_before, &head_after));
    if ctx.globals.verbose && !ed.new_ops.is_empty() {
        println!("  {}", t!("stack.new_ops", ids = conj_list.join(", ")));
    }
    println!("{}", render::pooled_line(&ed.pool_adds));
    Ok(())
}

/// `opt <ids> -f`：显式弹出。自顶向下逐个：置换升顶 → 出栈入池；head 回退重算并物化。
/// res = Some 时冲突经交换文件内容解决（步骤回放）。
pub(crate) fn opt_pop(ctx: &Ctx, a: OptArgs, res: Option<&Resolver<'_>>) -> Res<()> {
    if a.include.is_some() {
        return Err(PocError::Usage(t!("error.pop_no_include").to_string()));
    }
    if a.message.is_some() {
        return Err(PocError::Usage(t!("error.pop_no_message").to_string()));
    }
    let (compose_name, st0) = require_current(ctx)?;
    if st0.ops.is_empty() {
        return Err(PocError::Msg(t!("error.stack_empty").to_string()));
    }
    let mut ids: Vec<Hash> = Vec::new();
    for s in &a.ids {
        ids.push(resolve_op(ctx, s)?.0);
    }
    let mut ed = StackEdit::load(ctx, &st0)?;
    let mut order: Vec<Hash> = Vec::new();
    for h in &ids {
        match ed.canon(h) {
            Some(o) => order.push(o),
            None => {
                return Err(PocError::Msg(
                    t!(
                        "error.pop_not_on_stack",
                        id = paint(Token::Id, h.short()),
                        name = compose_name
                    )
                    .to_string(),
                ));
            }
        }
    }
    order.sort_by(|x, y| ed.pos[y].cmp(&ed.pos[x])); // 自顶向下
    let work = require_clean(ctx, &st0.head)?;
    let old_head = ed.head;
    for id in &order {
        if !ed.lift_to_top(ctx, id, res)? {
            break;
        }
        ed.pop_top(id);
    }
    if !ed.pending.is_empty() || !ed.binaries.is_empty() {
        return finish_conflict(
            ctx,
            crate::step::KIND_POP,
            &a.ids,
            "",
            "",
            &[(compose_name.as_str(), &st0.head)],
            &ed.pending,
            &ed.binaries,
        );
    }
    let list: Vec<String> = ids.iter().map(|h| h.short()).collect();
    let ev = Event::new(
        EVT_POP,
        &compose_name,
        Some(ids[0]),
        "",
        &format!("popped {}", list.join(" ")),
        &format!("head -> {}", ed.head.short()),
        now_ms(),
    );
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.pop",
                ids = list.join(" "),
                from = paint(Token::Id, old_head.short()),
                to = paint(Token::Id, ed.head.short())
            )
        );
        return Ok(());
    }
    ctx.store.commit_rewrite(
        &compose_name,
        &ed.state(),
        &ed.new_ops,
        &ed.new_trees,
        &ed.new_blobs,
        &ed.pool_adds,
        &ev,
    )?;
    let target = ctx
        .store
        .get_tree(&ed.head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    tree::materialize(&ctx.root, &target, &ctx.store, &work)?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, list.join(" ")));
        return Ok(());
    }
    println!(
        "{}",
        t!("stack.popped", ids = list.join(" "), name = compose_name)
    );
    println!("{}", render::head_line(&old_head, &ed.head));
    println!("{}", render::pooled_line(&ed.pool_adds));
    Ok(())
}

/// 连续段精确复合：c =（段首.pre, 段末.post）原位替换 [lo..=hi]，head 不变，原段入池。
fn compact_contiguous(
    ctx: &Ctx,
    compose_name: &str,
    st0: &ComposeState,
    lo: usize,
    hi: usize,
    message: Option<String>,
) -> Res<()> {
    let old_head = st0.head;
    require_clean(ctx, &old_head)?;
    let span: Vec<Hash> = st0.ops[lo..=hi].to_vec();
    let first = ctx
        .store
        .get_op(&span[0])?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
    let last = ctx
        .store
        .get_op(&span[span.len() - 1])?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
    let (author, _) = config::resolve_identity(&ctx.store)?;
    let msg = take_message(ctx, message)?;
    let c = Op {
        pre: first.pre,
        post: last.post,
        author: author.clone(),
        msg: msg.clone(),
        time_ms: now_ms(),
    };
    let cid = Hash::compute(TAG_OP, &c.encode());
    let mut new_ops: Vec<Hash> = st0.ops[..lo].to_vec();
    new_ops.push(cid);
    new_ops.extend_from_slice(&st0.ops[hi + 1..]);
    let mut st = st0.clone();
    st.ops = new_ops; // head 不变：段首 pre 接段末 post，复合总值不变
    let mut new_origs: Vec<Hash> = st0.origs[..lo].to_vec();
    new_origs.push(st0.origs[lo]);
    new_origs.extend_from_slice(&st0.origs[hi + 1..]);
    st.origs = new_origs;
    let shorts: Vec<String> = span.iter().map(|h| h.short()).collect();
    let ev = Event::new(
        EVT_COMPACT,
        compose_name,
        Some(cid),
        &author.name,
        &msg,
        &format!("compacted {} in place; originals pooled", shorts.join(" ")),
        c.time_ms,
    );
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.compact_inplace",
                ids = shorts.join(" "),
                id = paint(Token::Id, cid.short()),
                msg = msg,
                head = paint(Token::Id, old_head.short())
            )
        );
        return Ok(());
    }
    ctx.store.compact_span(compose_name, &st, &c, &span, &ev)?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, cid.short()));
        return Ok(());
    }
    println!(
        "{}",
        t!(
            "stack.compacted",
            ids = shorts.join(" "),
            id = paint(Token::Id, cid.short()),
            msg = msg,
            mode = t!("stack.mode_inplace")
        )
    );
    println!("{}", render::head_line(&old_head, &st0.head));
    println!("{}", render::pooled_line(&span));
    Ok(())
}

/// `opt <ids> -c`：压缩为一个新 operation。
/// 连续段 → 精确复合原位替换（head 不变）；跨段 → 置换聚拢后复合（head 不变），
/// 聚拢冲突 → 步骤协议；含池内/他栈 → 按给定顺序复合，结果入池。
pub(crate) fn opt_compact(ctx: &Ctx, a: OptArgs, res: Option<&Resolver<'_>>) -> Res<()> {
    if a.include.is_some() {
        return Err(PocError::Usage(t!("error.compact_no_include").to_string()));
    }
    let (compose_name, st0) = require_current(ctx)?;
    let mut resolved: Vec<(Hash, Op)> = Vec::new();
    for s in &a.ids {
        resolved.push(resolve_op(ctx, s)?);
    }
    let mut seen = std::collections::HashSet::new();
    resolved.retain(|(h, _)| seen.insert(*h));
    if resolved.len() < 2 {
        return Err(PocError::Usage(t!("error.compact_need_two").to_string()));
    }
    let (author, _) = config::resolve_identity(&ctx.store)?;
    let msg = take_message(ctx, a.message.clone())?;
    let ed0 = StackEdit::load(ctx, &st0)?;
    // 分支选择必须走 canon（含"当前形态 id 归槽"）：只用 pos 会把 amend 后的
    // 栈内 op 误判成他栈/池内（QA 发现，2026-10-02）
    let on_stack: Vec<Hash> = resolved.iter().filter_map(|(h, _)| ed0.canon(h)).collect();

    if on_stack.len() == resolved.len() {
        // 全在当前栈上
        let mut ps: Vec<usize> = on_stack.iter().map(|h| ed0.pos[h]).collect();
        ps.sort_unstable();
        let lo = ps[0];
        let hi = ps[ps.len() - 1];
        if hi - lo + 1 == ps.len() {
            return compact_contiguous(ctx, &compose_name, &st0, lo, hi, a.message);
        }
        // 跨段：置换聚拢（sel 按栈序）；冲突 → 步骤协议
        require_clean(ctx, &st0.head)?;
        let mut ed = StackEdit::load(ctx, &st0)?;
        let old_head = ed.head;
        let sel: Vec<Hash> = ps.iter().map(|p| ed.orig[*p]).collect();
        if !ed.gather(ctx, &sel, res)? {
            // 置换冲突 → 步骤协议（§2.5：任何一段失败 = 进入步骤协议，不落地）
            return finish_conflict(
                ctx,
                crate::step::KIND_COMPACT,
                &a.ids,
                &msg,
                "",
                &[(compose_name.as_str(), &st0.head)],
                &ed.pending,
                &ed.binaries,
            );
        }
        {
            let q = ed.pos[&sel[0]];
            let k = sel.len();
            let cpre = ed.ops[q].1.pre;
            let cpost = ed.ops[q + k - 1].1.post;
            let c = Op {
                pre: cpre,
                post: cpost,
                author: author.clone(),
                msg: msg.clone(),
                time_ms: now_ms(),
            };
            let cid = Hash::compute(TAG_OP, &c.encode());
            ed.seal_block(q, k, cid, c.clone());
            let shorts: Vec<String> = sel.iter().map(|h| h.short()).collect();
            let ev = Event::new(
                EVT_COMPACT,
                &compose_name,
                Some(cid),
                &author.name,
                &msg,
                &format!("compacted {} gathered; originals pooled", shorts.join(" ")),
                c.time_ms,
            );
            if ctx.globals.dry_run {
                println!(
                    "{}",
                    t!(
                        "dryrun.compact_gather",
                        ids = shorts.join(" "),
                        id = paint(Token::Id, cid.short()),
                        msg = msg,
                        head = paint(Token::Id, old_head.short())
                    )
                );
                return Ok(());
            }
            ctx.store.commit_rewrite(
                &compose_name,
                &ed.state(),
                &ed.new_ops,
                &ed.new_trees,
                &ed.new_blobs,
                &ed.pool_adds,
                &ev,
            )?;
            if ctx.globals.quiet {
                println!("{}", paint(Token::Id, cid.short()));
                return Ok(());
            }
            println!(
                "{}",
                t!(
                    "stack.compacted",
                    ids = shorts.join(" "),
                    id = paint(Token::Id, cid.short()),
                    msg = msg,
                    mode = t!("stack.mode_gathered")
                )
            );
            println!("{}", render::head_line(&old_head, &ed.head));
            println!("{}", render::pooled_line(&ed.pool_adds));
            Ok(())
        }
    } else {
        // 含池内/他栈 opt：按给定顺序做对象级复合，结果入自由池；栈分毫不动
        let mut run_post = resolved[0].1.post;
        let mut trees: BTreeMap<Hash, Tree> = BTreeMap::new();
        let mut blobs: BTreeMap<Hash, Vec<u8>> = BTreeMap::new();
        for (fold_seq, (_, op)) in resolved[1..].iter().enumerate() {
            let base_t = ctx
                .store
                .get_tree(&op.pre)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))?;
            let ours_t = match trees.get(&run_post) {
                Some(t) => t.clone(),
                None => ctx
                    .store
                    .get_tree(&run_post)?
                    .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))?,
            };
            let theirs_t = ctx
                .store
                .get_tree(&op.post)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))?;
            let my_seq = fold_seq;
            let mut sub = |path: &str, fc: &merge::FileConflict| -> Option<Vec<u8>> {
                match res {
                    Some(f) => f(my_seq, path, fc),
                    None => None,
                }
            };
            let m = match merge::graft_resolving(&ctx.store, &base_t, &theirs_t, &ours_t, &mut sub)
            {
                Ok(m) => m,
                Err(fail) => {
                    let mut pending: Vec<(u64, String, Vec<u8>)> = Vec::new();
                    for fc in &fail.conflicts {
                        pending.push((my_seq as u64, fc.path.clone(), fc.marked.clone()));
                    }
                    return finish_conflict(
                        ctx,
                        crate::step::KIND_COMPACT,
                        &a.ids,
                        &msg,
                        "",
                        &[(compose_name.as_str(), &st0.head)],
                        &pending,
                        &fail.binaries,
                    );
                }
            };
            trees.insert(m.tree_id, m.tree);
            blobs.extend(m.blobs);
            run_post = m.tree_id;
        }
        let c = Op {
            pre: resolved[0].1.pre,
            post: run_post,
            author: author.clone(),
            msg: msg.clone(),
            time_ms: now_ms(),
        };
        let cid = Hash::compute(TAG_OP, &c.encode());
        let ev_compose = if on_stack.is_empty() {
            POOL_SCOPE.to_string()
        } else {
            compose_name.clone()
        };
        let ev = Event::new(
            EVT_COMPACT,
            &ev_compose,
            Some(cid),
            &author.name,
            &msg,
            &format!(
                "composed {} into the free pool; stack untouched",
                a.ids.len()
            ),
            c.time_ms,
        );
        if ctx.globals.dry_run {
            println!(
                "{}",
                t!(
                    "dryrun.compact_pool",
                    ids = a.ids.join(" "),
                    id = paint(Token::Id, cid.short()),
                    msg = msg
                )
            );
            return Ok(());
        }
        let pools = vec![cid];
        ctx.store.commit_rewrite(
            &compose_name,
            &st0,
            &[(cid, c.clone())],
            &trees,
            &blobs,
            &pools,
            &ev,
        )?;
        if ctx.globals.quiet {
            println!("{}", paint(Token::Id, cid.short()));
            return Ok(());
        }
        println!(
            "{}",
            t!(
                "stack.composite",
                id = paint(Token::Id, cid.short()),
                msg = msg
            )
        );
        println!("{}", render::head_line(&st0.head, &st0.head));
        println!("{}", render::pooled_line(&[cid]));
        Ok(())
    }
}

// ---------- show ----------

pub fn cmd_show(ctx: &Ctx, composes: bool, operations: bool, id: Option<String>) -> Res<()> {
    if let Some(id) = id {
        if composes || operations {
            return Err(PocError::Usage(t!("error.show_exclusive").to_string()));
        }
        return show_op_detail(ctx, &id);
    }
    if operations {
        return show_all_ops(ctx);
    }
    if composes {
        return show_composes(ctx);
    }
    // 默认：当前 compose 头 + 独立 stack 小节（块式，层级见 render.rs）
    match ctx.store.current_name()? {
        Some(name) => {
            let st = ctx.store.compose_get(&name)?.ok_or_else(|| {
                PocError::Msg(t!("error.compose_not_found", name = name).to_string())
            })?;
            let mut ops = Vec::new();
            for h in &st.ops {
                let op = ctx
                    .store
                    .get_op(h)?
                    .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
                ops.push((*h, op));
            }
            ui::page(
                ctx.globals.no_pager,
                &render::show_stack(&name, &st.base, &st.head, &ops),
            );
        }
        None => println!("{}", paint(Token::Dim, t!("status.no_compose"))),
    }
    Ok(())
}

fn show_composes(ctx: &Ctx) -> Res<()> {
    let current = ctx.store.current_name()?;
    let mut out = String::new();
    let mut first = true;
    for name in ctx.store.compose_names()? {
        let st = ctx
            .store
            .compose_get(&name)?
            .ok_or_else(|| PocError::Msg("数据不一致".into()))?;
        if !first {
            out.push('\n');
        }
        first = false;
        let cur = current.as_deref() == Some(name.as_str());
        let note = if st.ops.is_empty() {
            t!("show.empty_stack_note").to_string()
        } else {
            t!("show.n_ops", n = st.ops.len()).to_string()
        };
        out.push_str(&render::compose_block(
            &name, &st.base, &st.head, cur, &note,
        ));
    }
    ui::page(ctx.globals.no_pager, &out);
    Ok(())
}

fn show_all_ops(ctx: &Ctx) -> Res<()> {
    let mut ops = ctx.store.list_ops()?;
    ops.sort_by_key(|(_, o)| std::cmp::Reverse(o.time_ms));
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        paint(Token::Header, t!("show.ops_header", n = ops.len()))
    ));
    for (h, op) in &ops {
        out.push_str(&render::op_entry(h, op));
        out.push('\n');
    }
    ui::page(ctx.globals.no_pager, &out);
    Ok(())
}

pub fn resolve_op(ctx: &Ctx, s: &str) -> Res<(Hash, Op)> {
    let s = s.trim();
    if s.len() == 64 {
        let h = Hash::from_hex(s)?;
        let op = ctx
            .store
            .get_op(&h)?
            .ok_or_else(|| PocError::NotFound(t!("error.op_not_found", id = s).to_string()))?;
        return Ok((h, op));
    }
    if s.len() < 4 {
        return Err(PocError::Usage(t!("error.id_prefix_short").to_string()));
    }
    let all = ctx.store.list_ops()?;
    let m: Vec<&(Hash, Op)> = all.iter().filter(|(h, _)| h.has_prefix(s)).collect();
    match m.len() {
        0 => Err(PocError::NotFound(
            t!("error.op_not_found", id = s).to_string(),
        )),
        1 => Ok((m[0].0, m[0].1.clone())),
        _ => {
            let list: Vec<String> = m.iter().take(6).map(|(h, _)| h.short()).collect();
            Err(PocError::Msg(
                t!("error.op_ambiguous", id = s, list = list.join(", ")).to_string(),
            ))
        }
    }
}

fn show_op_detail(ctx: &Ctx, id_str: &str) -> Res<()> {
    let (h, op) = resolve_op(ctx, id_str)?;
    let pre = ctx
        .store
        .get_tree(&op.pre)?
        .ok_or_else(|| PocError::Msg("前态树缺失".into()))?;
    let post = ctx
        .store
        .get_tree(&op.post)?
        .ok_or_else(|| PocError::Msg("后态树缺失".into()))?;
    let diff = tree_diff_text(ctx, &pre, &post, false)?;
    ui::page(ctx.globals.no_pager, &render::op_detail(&h, &op, &diff));
    Ok(())
}

// ---------- log ----------

/// `poc log`：按栈回放追加式操作日志（§5.7）。删栈不删史。
pub fn cmd_log(ctx: &Ctx, name: Option<String>, all: bool) -> Res<()> {
    if all && name.is_some() {
        return Err(PocError::Usage(t!("error.log_all_name").to_string()));
    }
    let entries = ctx.store.log_list()?;
    if entries.is_empty() {
        println!("{}", t!("log.empty"));
        return Ok(());
    }
    if all {
        let mut order: Vec<String> = Vec::new();
        let mut groups: BTreeMap<String, Vec<(u64, Event)>> = BTreeMap::new();
        for (seq, ev) in entries {
            if !groups.contains_key(&ev.compose) {
                order.push(ev.compose.clone());
            }
            groups
                .entry(ev.compose.clone())
                .or_default()
                .push((seq, ev));
        }
        let mut out = String::new();
        let mut first = true;
        for n in order {
            if !first {
                out.push('\n');
            }
            first = false;
            out.push_str(&format!(
                "{}\n",
                paint(Token::Header, t!("log.history_header", name = n))
            ));
            for (seq, ev) in &groups[&n] {
                out.push_str(&ev.line(*seq));
                out.push('\n');
            }
        }
        ui::page(ctx.globals.no_pager, &out);
        return Ok(());
    }
    let target = match name {
        Some(n) => n,
        None => ctx
            .store
            .current_name()?
            .ok_or_else(|| PocError::Msg(t!("error.no_current_log").into()))?,
    };
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        paint(Token::Header, t!("log.history_header", name = target))
    ));
    let mut any = false;
    for (seq, ev) in &entries {
        if ev.compose == target {
            any = true;
            out.push_str(&ev.line(*seq));
            out.push('\n');
        }
    }
    if !any {
        out.push_str(&format!("  {}\n", paint(Token::Dim, t!("log.none"))));
    }
    ui::page(ctx.globals.no_pager, &out);
    Ok(())
}

// ---------- diff ----------

pub fn cmd_diff(ctx: &Ctx, a: Option<String>, b: Option<String>, stat: bool) -> Res<()> {
    match (a, b) {
        (None, None) => {
            let (_, st) = require_current(ctx)?;
            let head_tree = ctx
                .store
                .get_tree(&st.head)?
                .ok_or_else(|| PocError::Msg("head 树缺失".into()))?;
            let work = tree::scan(&ctx.root)?;
            // 工作区未记录内容不在 store，新侧直接取内存
            let text = worktree_diff_text(ctx, &head_tree, &work, stat)?;
            ui::page(ctx.globals.no_pager, &text);
        }
        (Some(x), None) => {
            let (_, op) = resolve_op(ctx, &x)?;
            let pre = ctx
                .store
                .get_tree(&op.pre)?
                .ok_or_else(|| PocError::Msg("前态树缺失".into()))?;
            let post = ctx
                .store
                .get_tree(&op.post)?
                .ok_or_else(|| PocError::Msg("后态树缺失".into()))?;
            let text = tree_diff_text(ctx, &pre, &post, stat)?;
            ui::page(ctx.globals.no_pager, &text);
        }
        (Some(x), Some(y)) => {
            let (_, ox) = resolve_op(ctx, &x)?;
            let (_, oy) = resolve_op(ctx, &y)?;
            let tx = ctx
                .store
                .get_tree(&ox.post)?
                .ok_or_else(|| PocError::Msg("快照缺失".into()))?;
            let ty = ctx
                .store
                .get_tree(&oy.post)?
                .ok_or_else(|| PocError::Msg("快照缺失".into()))?;
            let text = tree_diff_text(ctx, &tx, &ty, stat)?;
            ui::page(ctx.globals.no_pager, &text);
        }
        (None, Some(_)) => return Err(PocError::Usage(t!("error.diff_pair").to_string())),
    }
    Ok(())
}

/// 两个已入库快照间的展示文本：新增/删除/修改逐文件 unified（或 --stat 统计）。
fn tree_diff_text(ctx: &Ctx, old: &Tree, new: &Tree, stat: bool) -> Res<String> {
    diff_text(ctx, old, NewSide::Tree(new), stat)
}

/// 工作区 vs 快照：新侧内容取自内存（未记录内容从没入过 store）。
fn worktree_diff_text(ctx: &Ctx, old: &Tree, work: &WorkMap, stat: bool) -> Res<String> {
    diff_text(ctx, old, NewSide::Work(work), stat)
}

enum NewSide<'a> {
    Tree(&'a Tree),
    Work(&'a WorkMap),
}

fn diff_text(ctx: &Ctx, old: &Tree, new: NewSide<'_>, stat: bool) -> Res<String> {
    let om = old.to_map();
    let nm = match &new {
        NewSide::Tree(t) => t.to_map(),
        NewSide::Work(w) => w
            .iter()
            .map(|(p, wi)| (p.clone(), (wi.mode, tree::hash_of(wi))))
            .collect(),
    };
    let d = diff_maps(&om, &nm);
    if d.is_empty() {
        return Ok(format!("{}\n", t!("diff.none")));
    }
    let new_bytes = |p: &str| -> Res<Option<Vec<u8>>> {
        match new {
            NewSide::Tree(_) => blob_of_entry(ctx, &nm, p),
            NewSide::Work(w) => Ok(w.get(p).map(|wi| wi.data.clone())),
        }
    };
    // 先收集（路径, 旧内容, 新内容）再渲染：--stat 的路径列按最长路径对齐
    type DiffRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);
    let mut rows: Vec<DiffRow> = Vec::new();
    for p in &d.added {
        rows.push((p.clone(), None, new_bytes(p)?));
    }
    for p in &d.removed {
        rows.push((p.clone(), blob_of_entry(ctx, &om, p)?, None));
    }
    for p in &d.modified {
        rows.push((p.clone(), blob_of_entry(ctx, &om, p)?, new_bytes(p)?));
    }
    let mut out = String::new();
    if stat {
        let w = rows
            .iter()
            .map(|(p, ..)| render::disp_width(p))
            .max()
            .unwrap_or(0);
        for (p, ob, nb) in rows {
            let ob = ob.unwrap_or_default();
            let nb = nb.unwrap_or_default();
            if diff::is_binary(&ob) || diff::is_binary(&nb) {
                out.push_str(&format!(
                    "  {} | {}\n",
                    render::pad_disp(&p, w),
                    paint(Token::Dim, t!("diff.binary"))
                ));
            } else {
                let (a, r) = diff::stat_lines(&ob, &nb);
                out.push_str(&format!(
                    "  {} | {} {}\n",
                    render::pad_disp(&p, w),
                    paint(Token::Added, format!("+{a}")),
                    paint(Token::Removed, format!("-{r}"))
                ));
            }
        }
    } else {
        for (p, ob, nb) in rows {
            let ob = ob.unwrap_or_default();
            let nb = nb.unwrap_or_default();
            if diff::is_binary(&ob) || diff::is_binary(&nb) {
                out.push_str(&format!("{}\n", t!("diff.binary_full")));
            } else {
                out.push_str(&colorize_unified(&diff::unified(
                    &ob,
                    &nb,
                    &format!("a/{p}"),
                    &format!("b/{p}"),
                )));
            }
        }
    }
    Ok(out)
}

/// unified 差异上色：+ 行绿、- 行红、@@ 头暗淡；正文不动，保持可复制。
fn colorize_unified(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let colored = if body.starts_with('+') {
            paint(Token::Added, body)
        } else if body.starts_with('-') {
            paint(Token::Removed, body)
        } else if body.starts_with("@@") {
            paint(Token::Dim, body)
        } else {
            body.to_string()
        };
        out.push_str(&colored);
        out.push('\n');
    }
    out
}

fn blob_of_entry(ctx: &Ctx, m: &BTreeMap<String, (Mode, Hash)>, p: &str) -> Res<Option<Vec<u8>>> {
    match m.get(p) {
        Some((_, h)) => ctx.store.get_blob(h),
        None => Ok(None),
    }
}

// ---------- cmp ----------

fn validate_compose_name(name: &str) -> Res<()> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err(PocError::Usage(
            t!("error.bad_compose_name", name = name).to_string(),
        ));
    }
    Ok(())
}

pub fn cmd_cmp(
    ctx: &Ctx,
    names: Vec<String>,
    compact: bool,
    new: Option<String>,
    fork: Option<String>,
) -> Res<()> {
    crate::step::gate(&ctx.store)?;
    if compact {
        if fork.is_some() {
            return Err(PocError::Usage(t!("error.fork_no_compact").to_string()));
        }
        if names.len() < 2 {
            return Err(PocError::Usage(t!("error.merge_min_two").to_string()));
        }
        // `-N` 在合并形态下给出结果栈名（缺省 merged-N）
        return cmd_cmp_merge(ctx, &names, new.as_deref(), None);
    }
    if let Some(name) = new {
        if !names.is_empty() {
            return Err(PocError::Usage(t!("error.new_no_names").to_string()));
        }
        return cmd_cmp_new(ctx, name, fork);
    }
    if fork.is_some() {
        return Err(PocError::Usage(t!("error.fork_needs_new").to_string()));
    }
    if names.is_empty() {
        return Err(PocError::Usage(t!("error.cmp_usage").to_string()));
    }
    if names.len() != 1 {
        return Err(PocError::Usage(t!("error.switch_one").to_string()));
    }
    cmd_cmp_switch(ctx, names[0].clone())
}

/// `cmp <names> -c`：合并为**一个新建 Compose（副本）**——O2 构造：公共前缀（共享
/// opt 对象）+ B 独有段逐个嫁接到 A.head。被合并栈保留、当前栈不切换（§5.3）。
/// res = Some 时冲突经交换文件内容解决（步骤回放）。
pub(crate) fn cmd_cmp_merge(
    ctx: &Ctx,
    names: &[String],
    result: Option<&str>,
    res: Option<&Resolver<'_>>,
) -> Res<()> {
    let result_name = match result {
        Some(n) => {
            validate_compose_name(n)?;
            if ctx.store.compose_get(n)?.is_some() {
                return Err(PocError::Msg(
                    t!("error.compose_exists", name = n).to_string(),
                ));
            }
            n.to_string()
        }
        None => {
            let mut i = 1usize;
            loop {
                let cand = if i == 1 {
                    "merged".to_string()
                } else {
                    format!("merged-{i}")
                };
                if ctx.store.compose_get(&cand)?.is_none() {
                    break cand;
                }
                i += 1;
            }
        }
    };
    let mut acc = ctx.store.compose_get(&names[0])?.ok_or_else(|| {
        PocError::NotFound(t!("error.compose_not_found", name = names[0]).to_string())
    })?;
    let mut new_ops: Vec<(Hash, Op)> = Vec::new();
    let mut new_trees: BTreeMap<Hash, Tree> = BTreeMap::new();
    let mut new_blobs: BTreeMap<Hash, Vec<u8>> = BTreeMap::new();
    let mut mseq: usize = 0;
    for nxt in &names[1..] {
        let bst = ctx.store.compose_get(nxt)?.ok_or_else(|| {
            PocError::NotFound(t!("error.compose_not_found", name = nxt).to_string())
        })?;
        // 公共前缀：共享的 opt 对象不重铸
        let k = acc
            .ops
            .iter()
            .zip(bst.ops.iter())
            .take_while(|(x, y)| x == y)
            .count();
        if k == 0 && acc.base != bst.base {
            return Err(PocError::Msg(
                t!("error.no_common_ancestor", a = names[0], b = nxt).to_string(),
            ));
        }
        let mut head = acc.head;
        for opid in &bst.ops[k..] {
            let op = ctx
                .store
                .get_op(opid)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
            let base_t = ctx
                .store
                .get_tree(&op.pre)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))?;
            let ours_t = match new_trees.get(&head) {
                Some(t) => t.clone(),
                None => ctx
                    .store
                    .get_tree(&head)?
                    .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?,
            };
            let theirs_t = ctx
                .store
                .get_tree(&op.post)?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_tree").to_string()))?;
            let my_seq = mseq;
            let mut sub = |path: &str, fc: &merge::FileConflict| -> Option<Vec<u8>> {
                match res {
                    Some(f) => f(my_seq, path, fc),
                    None => None,
                }
            };
            let m = match merge::graft_resolving(&ctx.store, &base_t, &theirs_t, &ours_t, &mut sub)
            {
                Ok(m) => m,
                Err(fail) => {
                    // 冲突 → 步骤协议：守卫 = 各源栈当前 head
                    let mut gs: Vec<(String, Hash)> = Vec::new();
                    for n in names {
                        if let Some(st) = ctx.store.compose_get(n)? {
                            gs.push((n.clone(), st.head));
                        }
                    }
                    let guards: Vec<(&str, &Hash)> =
                        gs.iter().map(|(n, h)| (n.as_str(), h)).collect();
                    let mut pending: Vec<(u64, String, Vec<u8>)> = Vec::new();
                    for fc in &fail.conflicts {
                        pending.push((my_seq as u64, fc.path.clone(), fc.marked.clone()));
                    }
                    return finish_conflict(
                        ctx,
                        crate::step::KIND_MERGE,
                        names,
                        "",
                        &result_name,
                        &guards,
                        &pending,
                        &fail.binaries,
                    );
                }
            };
            mseq += 1;
            new_trees.insert(m.tree_id, m.tree);
            for (h, b) in m.blobs {
                new_blobs.insert(h, b);
            }
            let conj = Op {
                pre: head,
                post: m.tree_id,
                author: op.author,
                msg: op.msg,
                time_ms: op.time_ms,
            };
            let cid = Hash::compute(TAG_OP, &conj.encode());
            new_ops.push((cid, conj));
            head = m.tree_id;
            acc.ops.push(cid);
            acc.origs.push(*opid);
        }
        acc.head = head;
    }
    let ev = Event::new(
        EVT_MERGE,
        &result_name,
        None,
        "",
        &format!("merged {} (copy)", names.join(" + ")),
        &format!("head {}, {} slots", acc.head.short(), acc.ops.len()),
        now_ms(),
    );
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.merge",
                names = names.join(" + "),
                name = result_name,
                id = paint(Token::Id, acc.head.short())
            )
        );
        return Ok(());
    }
    ctx.store
        .commit_new_compose(&result_name, &acc, &new_ops, &new_trees, &new_blobs, &ev)?;
    if ctx.globals.quiet {
        return Ok(());
    }
    println!(
        "{}",
        t!("cmp.merged", names = names.join(" + "), name = result_name)
    );
    println!(
        "  {}",
        t!("stack.head_only", id = paint(Token::Id, acc.head.short()))
    );
    println!(
        "  {}",
        paint(Token::Dim, t!("cmp.merge_hint", name = result_name))
    );
    Ok(())
}

pub(crate) fn cmd_cmp_new(ctx: &Ctx, name: String, fork: Option<String>) -> Res<()> {
    validate_compose_name(&name)?;
    let (cur_name, cur) = require_current(ctx)?;
    if ctx.store.compose_get(&name)?.is_some() {
        return Err(PocError::Msg(
            t!("error.compose_exists", name = name).to_string(),
        ));
    }
    let (st, detail, cut_op) = match fork.as_deref() {
        None => (
            ComposeState {
                base: cur.head,
                head: cur.head,
                ops: vec![],
                origs: vec![],
            },
            t!("fork.detail_new", name = cur_name).to_string(),
            None,
        ),
        Some("") => (
            ComposeState {
                base: cur.base,
                head: cur.head,
                ops: cur.ops.clone(),
                origs: cur.origs.clone(),
            },
            t!("fork.detail_fork", name = cur_name).to_string(),
            None,
        ),
        Some(idstr) => {
            let (h, _) = resolve_op(ctx, idstr)?;
            let pos = cur
                .origs
                .iter()
                .position(|x| *x == h)
                .or_else(|| cur.ops.iter().position(|x| *x == h))
                .ok_or_else(|| {
                    PocError::Msg(
                        t!("error.fork_not_on_stack", id = h.short(), name = cur_name).to_string(),
                    )
                })?;
            let cut = ctx
                .store
                .get_op(&cur.ops[pos])?
                .ok_or_else(|| PocError::Msg(t!("error.data_missing_op").to_string()))?;
            let st = ComposeState {
                base: cur.base,
                head: cut.post,
                ops: cur.ops[..=pos].to_vec(),
                origs: cur.origs[..=pos].to_vec(),
            };
            (
                st,
                t!(
                    "fork.detail_prefix",
                    name = cur_name,
                    opid = cur.ops[pos].short(),
                    n = pos + 1
                )
                .to_string(),
                Some(cur.ops[pos]),
            )
        }
    };
    let head_moves = st.head != cur.head;
    let work = if head_moves {
        Some(require_clean(ctx, &cur.head)?)
    } else {
        None
    };
    let ev = Event::new(
        EVT_CMP_NEW,
        &name,
        cut_op,
        "",
        "new compose + switch",
        &detail,
        now_ms(),
    );
    if ctx.globals.dry_run {
        println!(
            "{}",
            t!(
                "dryrun.new",
                name = name,
                detail = detail,
                id = paint(Token::Id, st.head.short())
            )
        );
        return Ok(());
    }
    ctx.store.compose_create(&name, &st, &ev)?;
    if let Some(work) = &work {
        let target = ctx
            .store
            .get_tree(&st.head)?
            .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
        tree::materialize(&ctx.root, &target, &ctx.store, work)?;
    }
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, st.head.short()));
        return Ok(());
    }
    println!("{}", t!("cmp.created", name = name, detail = detail));
    if head_moves {
        println!("{}", render::head_line(&cur.head, &st.head));
    }
    Ok(())
}

pub(crate) fn cmd_cmp_switch(ctx: &Ctx, name: String) -> Res<()> {
    validate_compose_name(&name)?;
    let st = ctx.store.compose_get(&name)?.ok_or_else(|| {
        PocError::NotFound(t!("error.compose_not_found", name = name).to_string())
    })?;
    let (cur_name, cur) = require_current(ctx)?;
    if name == cur_name {
        println!("{}", t!("cmp.already", name = name));
        return Ok(());
    }
    let cur_head_tree = ctx
        .store
        .get_tree(&cur.head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    let work = tree::scan(&ctx.root)?;
    if !tree::disk_matches_tree(&work, &cur_head_tree) {
        return Err(PocError::Dirty(t!("error.dirty_switch").to_string()));
    }
    let target_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg(t!("error.data_missing_head").to_string()))?;
    tree::materialize(&ctx.root, &target_tree, &ctx.store, &work)?;
    ctx.store.meta_set("current", &name)?;
    if ctx.globals.quiet {
        println!("{}", paint(Token::Id, st.head.short()));
        return Ok(());
    }
    println!(
        "{}",
        t!(
            "cmp.switched",
            name = name,
            id = paint(Token::Id, st.head.short())
        )
    );
    Ok(())
}

// ---------- status ----------

pub fn cmd_status(ctx: &Ctx) -> Res<()> {
    let mut out = String::new();
    // 自由池：入池时间前置（定宽），id、消息随后
    let mut pool_rows: Vec<(Hash, u64, String)> = Vec::new();
    for (h, t) in ctx.store.pool_list()? {
        let msg = ctx
            .store
            .get_op(&h)?
            .map(|o| o.msg)
            .unwrap_or_else(|| t!("status.missing").to_string());
        pool_rows.push((h, t, msg));
    }
    pool_rows.sort_by_key(|(_, t, _)| *t);
    out.push_str(&format!(
        "{}\n",
        paint(Token::Header, t!("status.pool_header", n = pool_rows.len()))
    ));
    if pool_rows.is_empty() {
        out.push_str(&format!(
            "  {}\n",
            paint(Token::Dim, t!("status.pool_empty"))
        ));
    }
    for (h, t, msg) in &pool_rows {
        out.push_str(&format!(
            "  {}  {}  \"{}\"\n",
            render::fmt_time(*t),
            paint(Token::Id, h.short()),
            msg
        ));
    }

    if let Some(name) = ctx.store.current_name()? {
        let st = ctx
            .store
            .compose_get(&name)?
            .ok_or_else(|| PocError::Msg(t!("error.compose_not_found", name = name).to_string()))?;
        out.push('\n');
        out.push_str(&format!("{}\n", render::compose_line(&name, true)));
        out.push_str(&format!(
            "  base:  {}\n  head:  {}\n",
            paint(Token::Id, st.base.short()),
            paint(Token::Id, st.head.short())
        ));
        let head_tree = ctx
            .store
            .get_tree(&st.head)?
            .ok_or_else(|| PocError::Msg("head 树缺失".into()))?;
        let head_map = head_tree.to_map();
        let work = tree::scan(&ctx.root)?;
        let work_map: BTreeMap<String, (Mode, Hash)> = work
            .iter()
            .map(|(p, w)| (p.clone(), (w.mode, Hash::compute(TAG_BLOB, &w.data))))
            .collect();
        let d = diff_maps(&head_map, &work_map);
        out.push('\n');
        if d.is_empty() {
            out.push_str(&format!("{}\n", paint(Token::Dim, t!("status.clean"))));
        } else {
            let mut parts: Vec<String> = Vec::new();
            if !d.added.is_empty() {
                parts.push(paint(
                    Token::Added,
                    t!("status.n_added", n = d.added.len()).to_string(),
                ));
            }
            if !d.removed.is_empty() {
                parts.push(paint(
                    Token::Removed,
                    t!("status.n_deleted", n = d.removed.len()).to_string(),
                ));
            }
            if !d.modified.is_empty() {
                parts.push(paint(
                    Token::Updated,
                    t!("status.n_updated", n = d.modified.len()).to_string(),
                ));
            }
            out.push_str(&format!(
                "{} {}\n",
                paint(Token::Header, t!("status.unrecorded_label")),
                parts.join(", ")
            ));
            for p in &d.added {
                out.push_str(&format!("  {}\n", paint(Token::Added, format!("+ {p}"))));
            }
            for p in &d.removed {
                out.push_str(&format!("  {}\n", paint(Token::Removed, format!("- {p}"))));
            }
            for p in &d.modified {
                out.push_str(&format!("  {}\n", paint(Token::Updated, format!("M {p}"))));
            }
        }
    } else {
        out.push('\n');
        out.push_str(&format!("{}\n", paint(Token::Dim, t!("status.no_compose"))));
    }

    out.push('\n');
    match crate::step::load(&ctx.store)? {
        Some(s) => out.push_str(&render::step_section(&s, &ctx.root)),
        None => out.push_str(&format!("{}\n", t!("status.pending_none"))),
    }
    ui::page(ctx.globals.no_pager, &out);
    Ok(())
}

// ---------- config ----------

pub fn cmd_config(ctx: &Ctx, unset: bool, key: Option<String>, value: Option<String>) -> Res<()> {
    match (unset, key, value) {
        (false, None, _) => {
            println!("{}", paint(Token::Header, t!("config.effective")));
            let keys = [config::USER_NAME, config::USER_EMAIL, "project.name"];
            let width = keys.iter().map(|k| k.len()).max().unwrap_or(0);
            for k in [config::USER_NAME, config::USER_EMAIL] {
                match config::resolve_key(&ctx.store, k)? {
                    Some((v, s)) => println!(
                        "  {:<width$}  {}  {}",
                        k,
                        v,
                        paint(Token::Dim, format!("({})", s.label())),
                        width = width
                    ),
                    None => {
                        println!("  {:<width$}  {}", k, t!("config.not_set"), width = width)
                    }
                }
            }
            let pname = ctx.store.meta_get("project.name")?.unwrap_or_default();
            println!(
                "  {:<width$}  {}  {}",
                "project.name",
                pname,
                paint(Token::Dim, format!("({})", t!("config.source_init"))),
                width = width
            );
            Ok(())
        }
        (true, Some(k), None) => {
            if !config::valid_config_key(&k) {
                return Err(PocError::Usage(
                    t!("config.err_bad_key", key = k).to_string(),
                ));
            }
            ctx.store.meta_del(&k)?;
            println!("{}", t!("config.removed", key = k));
            Ok(())
        }
        (false, Some(k), Some(v)) => {
            if !config::valid_config_key(&k) {
                return Err(PocError::Usage(
                    t!("config.err_bad_key", key = k).to_string(),
                ));
            }
            let v = v.trim().to_string();
            ctx.store.meta_set(&k, &v)?;
            println!("{}", t!("config.written", key = k, value = v));
            Ok(())
        }
        _ => Err(PocError::Usage(t!("config.err_usage").to_string())),
    }
}

// ---------- 步骤协议（§1.6 / §4.4）----------

/// `poc --commit`：以交换文件内容回放原操作（graft 链以交换文件内容为准）。
pub fn step_commit(ctx: &Ctx) -> Res<()> {
    let step = crate::step::load(&ctx.store)?
        .ok_or_else(|| PocError::Msg(t!("step.err_none").to_string()))?;
    // 前置状态指纹校验：源栈 head 必须与建步骤时一致
    for (name, headhex) in &step.guards {
        let st = ctx
            .store
            .compose_get(name)?
            .ok_or_else(|| PocError::Msg(t!("error.compose_not_found", name = name).to_string()))?;
        if st.head.hex() != *headhex {
            return Err(PocError::Msg(
                t!("step.err_guard_moved", name = name).to_string(),
            ));
        }
    }
    let map: HashMap<(u64, String), String> = step
        .swaps
        .iter()
        .map(|(s, p, f)| ((*s, p.clone()), f.clone()))
        .collect();
    let resolver = |seq: usize, path: &str, _fc: &merge::FileConflict| -> Option<Vec<u8>> {
        let fname = map.get(&(seq as u64, path.to_string()))?;
        let bytes = std::fs::read(crate::step::swap_root(&ctx.root).join(fname)).ok()?;
        if crate::step::has_markers(&bytes) {
            return None; // 仍有冲突标记 = 未解决
        }
        Some(bytes)
    };
    let res: Option<&Resolver<'_>> = Some(&resolver);
    let args = |ids: &[String], free: bool, compact: bool, message: Option<String>| OptArgs {
        ids: ids.to_vec(),
        message,
        include: None,
        free,
        compact,
        amend: false,
    };
    match step.kind {
        crate::step::KIND_LIFT => opt_lift(ctx, args(&step.ids, false, false, None), res)?,
        crate::step::KIND_POP => opt_pop(ctx, args(&step.ids, true, false, None), res)?,
        crate::step::KIND_COMPACT => opt_compact(
            ctx,
            args(&step.ids, false, true, Some(step.message.clone())),
            res,
        )?,
        crate::step::KIND_MERGE => cmd_cmp_merge(ctx, &step.ids, Some(&step.result_name), res)?,
        other => {
            return Err(PocError::Msg(
                t!("step.err_unknown_kind", kind = other).to_string(),
            ));
        }
    }
    crate::step::clear(&ctx.store, &ctx.root)?;
    println!("{}", t!("step.committed"));
    Ok(())
}

/// `poc --cancel`：放弃待办步骤（存储与工作区未被改动过）。
pub fn step_cancel(ctx: &Ctx) -> Res<()> {
    match crate::step::load(&ctx.store)? {
        None => Err(PocError::Msg(t!("step.err_none").to_string())),
        Some(s) => {
            let kind = crate::step::kind_label(s.kind);
            crate::step::clear(&ctx.store, &ctx.root)?;
            println!("{}", t!("step.cancelled", kind = kind));
            Ok(())
        }
    }
}

// ---------- gc ----------

pub fn cmd_gc(ctx: &Ctx) -> Res<()> {
    let (b, t, o) = ctx.store.gc_run()?;
    println!("{}", t!("gc.done", blobs = b, trees = t, ops = o));
    Ok(())
}
