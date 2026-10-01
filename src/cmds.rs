//! `poc` 主工具的命令实现（安全本地操作；销毁族在 dcmds，链接在 cpoc——均不在此）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use globset::Glob;

use crate::config;
use crate::db::{tree_id, ComposeState, Store};
use crate::diff::{self, diff_maps};
use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::object::{Mode, Op, Tree};
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
    let name = ctx.store.current_name()?.ok_or_else(|| {
        PocError::Msg("当前没有 Compose（已被移除）；用 `poc cmp -N <name>` 新建".into())
    })?;
    let st = ctx.store.compose_get(&name)?.ok_or_else(|| {
        PocError::Msg(format!("当前 Compose `{name}` 不存在（数据不一致）"))
    })?;
    Ok((name, st))
}

// ---------- proj ----------

pub fn cmd_proj(new_name: Option<String>, path: Option<String>) -> Res<()> {
    let target: PathBuf = match (new_name, path) {
        (Some(_), Some(_)) => return Err(PocError::Usage("`-N` 与 <path> 不可同时使用".to_string())),
        (Some(name), None) => {
            if name.is_empty() || name.contains('/') {
                return Err(PocError::Usage("非法项目名".to_string()));
            }
            let p = std::env::current_dir()?.join(&name);
            if p.exists() {
                return Err(PocError::Msg(format!("目录已存在：{}", p.display())));
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
        "已初始化 P.O.C. 项目：{}（Compose: main）",
        target.display()
    );
    Ok(())
}

pub fn init_at(root: &Path) -> Res<Store> {
    if Store::store_path(root).exists() {
        return Err(PocError::Msg(format!(
            "已是 P.O.C. 项目：{}",
            root.display()
        )));
    }
    let store = Store::create(root)?;
    let empty = Tree::default();
    let empty_id = tree_id(&empty);
    store.put_tree(&empty_id, &empty)?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    store.meta_set("project.name", &name)?;
    store.meta_set("current", "main")?;
    store.compose_put(
        "main",
        &ComposeState {
            base: empty_id,
            head: empty_id,
            ops: vec![],
        },
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
    if !a.ids.is_empty() {
        if a.amend {
            return Err(PocError::Usage(
                "`--amend` 不接受 operation ids（它改写栈顶）".to_string(),));
        }
        if a.free && a.compact {
            return Err(PocError::Usage("`-f` 与 `-c` 不可同时使用".to_string()));
        }
        return Err(PocError::Msg(
            "opt 提升 / -f 弹池 / -c 压缩需要行级 graft 原语（M2），尚未实现；当前可用：`poc opt -M` 记录、`poc opt --amend` 改写栈顶".to_string(),));
    }
    if a.free || a.compact {
        return Err(PocError::Usage("`-f`/`-c` 须与 operation ids 搭配".to_string()));
    }
    if a.amend {
        cmd_opt_amend(ctx, a)
    } else {
        cmd_opt_create(ctx, a)
    }
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
                .map_err(|e| PocError::Usage(format!("非法 glob：{e}")))?
                .compile_matcher();
            let mut m: WorkMap = WorkMap::new();
            for (p, w) in &work {
                if glob.is_match(p) {
                    m.insert(p.clone(), w.clone());
                }
            }
            for (p, (mode, bh)) in &head_tree.to_map() {
                if !glob.is_match(p) {
                    if let Some(b) = ctx.store.get_blob(bh)? {
                        m.insert(p.clone(), WorkItem { mode: *mode, data: b });
                    }
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
            return Err(PocError::Usage("消息不能为空".to_string()));
        }
        return Ok(m);
    }
    if ctx.globals.step {
        return Err(PocError::Msg("`-s` 模式不打开编辑器：请用 `-M` 给出消息".to_string()));
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
        return Err(PocError::Msg("消息为空，已取消记录".to_string()));
    }
    Ok(m)
}

fn hunk_total(
    ctx: &Ctx,
    head_map: &BTreeMap<String, (Mode, Hash)>,
    d: &diff::TreeDiff,
    candidate: &WorkMap,
) -> usize {
    let mut hunks = 0usize;
    for p in &d.modified {
        let old_b = head_map
            .get(p)
            .and_then(|(_, h)| ctx.store.get_blob(h).ok())
            .flatten()
            .unwrap_or_default();
        let new_b = candidate.get(p).map(|w| w.data.clone()).unwrap_or_default();
        hunks += diff::hunk_count(&old_b, &new_b);
    }
    hunks
}

fn print_record_summary(
    ctx: &Ctx,
    head_map: &BTreeMap<String, (Mode, Hash)>,
    d: &diff::TreeDiff,
    candidate: &WorkMap,
) -> Res<()> {
    let hunks = hunk_total(ctx, head_map, d, candidate);
    println!(
        "整体: +{} −{} 文件；内部: {} 文件修改（{} hunks）",
        d.added.len(),
        d.removed.len(),
        d.modified.len(),
        hunks
    );
    if ctx.globals.verbose {
        for p in &d.added {
            println!("  + {p}");
        }
        for p in &d.removed {
            println!("  − {p}");
        }
        for p in &d.modified {
            println!("  M {p}");
        }
    }
    Ok(())
}

fn cmd_opt_create(ctx: &Ctx, a: OptArgs) -> Res<()> {
    let (compose_name, mut st) = require_current(ctx)?;
    let head_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg("head 树缺失（数据不一致）".into()))?;
    let head_map = head_tree.to_map();
    let (candidate, tree, blobs) = build_candidate(ctx, &head_tree, a.include.as_deref())?;
    let d = diff_maps(&head_map, &tree.to_map());
    if d.is_empty() {
        return Err(PocError::Msg("没有可记录的变更（工作区与 head 一致）".to_string()));
    }
    let (author, _) = config::resolve_identity(&ctx.store)?;
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
        print_record_summary(ctx, &head_map, &d, &candidate)?;
        println!(
            "[dry-run] 将新建 opt {}  \"{}\"  压入 Compose `{compose_name}`（不落盘）",
            op_id.short(),
            op.msg
        );
        return Ok(());
    }
    st.ops.push(op_id);
    st.head = op.post;
    ctx.store
        .commit_op(&compose_name, &st, &op, &blobs, &tree)?;
    print_record_summary(ctx, &head_map, &d, &candidate)?;
    println!("opt {}  \"{}\"", op_id.short(), op.msg);
    Ok(())
}

fn cmd_opt_amend(ctx: &Ctx, a: OptArgs) -> Res<()> {
    let (compose_name, mut st) = require_current(ctx)?;
    let top_id = *st
        .ops
        .last()
        .ok_or_else(|| PocError::Msg("栈为空，无法 amend".to_string()))?;
    let top = ctx
        .store
        .get_op(&top_id)?
        .ok_or_else(|| PocError::Msg("栈顶对象缺失（数据不一致）".into()))?;
    let head_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg("head 树缺失（数据不一致）".into()))?;
    let (_candidate, tree, blobs) = build_candidate(ctx, &head_tree, a.include.as_deref())?;
    let (author, _) = config::resolve_identity(&ctx.store)?;
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
            "[dry-run] 将以（栈顶.pre, 工作区快照）改写栈顶 {} → {}（不落盘）",
            top_id.short(),
            op_id.short()
        );
        return Ok(());
    }
    st.ops.pop();
    st.ops.push(op_id);
    st.head = op.post;
    ctx.store
        .commit_op(&compose_name, &st, &op, &blobs, &tree)?;
    println!("已改写栈顶：{}  \"{}\"", op_id.short(), op.msg);
    Ok(())
}

// ---------- show ----------

pub fn cmd_show(ctx: &Ctx, composes: bool, operations: bool, id: Option<String>) -> Res<()> {
    if let Some(id) = id {
        if composes || operations {
            return Err(PocError::Usage("show <id> 与节选旗标互斥".to_string()));
        }
        return show_op_detail(ctx, &id);
    }
    if operations {
        return show_all_ops(ctx);
    }
    if composes {
        return show_composes(ctx);
    }
    // 默认：当前 Compose 名 + 栈（ASCII）
    match ctx.store.current_name()? {
        Some(name) => {
            let st = ctx
                .store
                .compose_get(&name)?
                .ok_or_else(|| PocError::Msg(format!("Compose `{name}` 不存在")))?;
            let mut ops = Vec::new();
            for h in &st.ops {
                let op = ctx
                    .store
                    .get_op(h)?
                    .ok_or_else(|| PocError::Msg(format!("opt 对象缺失：{}", h.short())))?;
                ops.push((*h, op));
            }
            print!("{}", render::stack_ascii(&name, &st.base, &st.head, &ops));
        }
        None => println!("（无当前 Compose）"),
    }
    Ok(())
}

fn show_composes(ctx: &Ctx) -> Res<()> {
    let current = ctx.store.current_name()?;
    for name in ctx.store.compose_names()? {
        let st = ctx
            .store
            .compose_get(&name)?
            .ok_or_else(|| PocError::Msg("数据不一致".into()))?;
        let cur = if current.as_deref() == Some(name.as_str()) {
            "  ← 当前"
        } else {
            ""
        };
        println!(
            "{}  base {}  head {}  栈深 {}{cur}",
            name,
            st.base.short(),
            st.head.short(),
            st.ops.len()
        );
    }
    Ok(())
}

fn show_all_ops(ctx: &Ctx) -> Res<()> {
    let mut ops = ctx.store.list_ops()?;
    ops.sort_by_key(|(_, o)| std::cmp::Reverse(o.time_ms));
    for (h, op) in ops {
        println!("{}", render::op_line(&h, &op));
    }
    Ok(())
}

pub fn resolve_op(ctx: &Ctx, s: &str) -> Res<(Hash, Op)> {
    let s = s.trim();
    if s.len() == 64 {
        let h = Hash::from_hex(s)?;
        let op = ctx
            .store
            .get_op(&h)?
            .ok_or_else(|| PocError::NotFound(format!("opt {s}")))?;
        return Ok((h, op));
    }
    if s.len() < 4 {
        return Err(PocError::Usage("id 前缀至少 4 位十六进制".to_string()));
    }
    let all = ctx.store.list_ops()?;
    let m: Vec<&(Hash, Op)> = all.iter().filter(|(h, _)| h.has_prefix(s)).collect();
    match m.len() {
        0 => Err(PocError::NotFound(format!("opt 前缀 {s}"))),
        1 => Ok((m[0].0, m[0].1.clone())),
        _ => {
            let list: Vec<String> = m.iter().take(6).map(|(h, _)| h.short()).collect();
            Err(PocError::Msg(format!(
                "id 前缀 {s} 命中多个：{}",
                list.join(", ")
            )))
        }
    }
}

fn show_op_detail(ctx: &Ctx, id_str: &str) -> Res<()> {
    let (h, op) = resolve_op(ctx, id_str)?;
    println!("opt {}", h.hex());
    println!("消息:   {}", op.msg);
    println!("作者:   {}", op.author.display());
    println!("时间:   {}", render::fmt_time(op.time_ms));
    println!("前态:   {}", op.pre.hex());
    println!("后态:   {}", op.post.hex());
    let pre = ctx
        .store
        .get_tree(&op.pre)?
        .ok_or_else(|| PocError::Msg("前态树缺失".into()))?;
    let post = ctx
        .store
        .get_tree(&op.post)?
        .ok_or_else(|| PocError::Msg("后态树缺失".into()))?;
    print!("\n{}", tree_diff_text(ctx, &pre, &post, false)?);
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
            let work_tree = tree::tree_of(&work);
            let text = tree_diff_text(ctx, &head_tree, &work_tree, stat)?;
            print!("{text}");
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
            print!("{text}");
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
            print!("{text}");
        }
        (None, Some(_)) => return Err(PocError::Usage("diff 需要两个 id 成对给出".to_string())),
    }
    Ok(())
}

/// 两个快照间的展示文本：新增/删除/修改逐文件 unified（或 --stat 统计）。
fn tree_diff_text(ctx: &Ctx, old: &Tree, new: &Tree, stat: bool) -> Res<String> {
    let om = old.to_map();
    let nm = new.to_map();
    let d = diff_maps(&om, &nm);
    let mut out = String::new();
    if d.is_empty() {
        return Ok("（无差异）\n".into());
    }
    let emit = |out: &mut String, p: &str, kind: char, ob: Option<Vec<u8>>, nb: Option<Vec<u8>>| {
        let ob = ob.unwrap_or_default();
        let nb = nb.unwrap_or_default();
        if stat {
            let (a, r) = diff::stat_lines(&ob, &nb);
            out.push_str(&format!("  {p} | +{a} −{r} {kind}\n"));
        } else {
            out.push_str(&diff::unified(&ob, &nb, &format!("a/{p}"), &format!("b/{p}")));
        }
    };
    for p in &d.added {
        let nb = blob_of_entry(ctx, &nm, p)?;
        emit(&mut out, p, '+', None, nb);
    }
    for p in &d.removed {
        let ob = blob_of_entry(ctx, &om, p)?;
        emit(&mut out, p, '−', ob, None);
    }
    for p in &d.modified {
        let ob = blob_of_entry(ctx, &om, p)?;
        let nb = blob_of_entry(ctx, &nm, p)?;
        emit(&mut out, p, 'M', ob, nb);
    }
    Ok(out)
}

fn blob_of_entry(
    ctx: &Ctx,
    m: &BTreeMap<String, (Mode, Hash)>,
    p: &str,
) -> Res<Option<Vec<u8>>> {
    match m.get(p) {
        Some((_, h)) => ctx.store.get_blob(h),
        None => Ok(None),
    }
}

// ---------- cmp ----------

fn validate_compose_name(name: &str) -> Res<()> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err(PocError::Usage(format!("非法 Compose 名：{name}")));
    }
    Ok(())
}

pub fn cmd_cmp(
    ctx: &Ctx,
    names: Vec<String>,
    compact: bool,
    new: Option<String>,
    fork: bool,
) -> Res<()> {
    if let Some(name) = new {
        if compact {
            return Err(PocError::Usage("`-c` 不与 `-N` 同用".to_string()));
        }
        if !names.is_empty() {
            return Err(PocError::Usage("`-N` 不与 names 同用".to_string()));
        }
        return cmd_cmp_new(ctx, name, fork);
    }
    if names.is_empty() {
        return Err(PocError::Usage(
            "poc cmp <name> 切换；poc cmp -N <name> [--fork] 新建；poc cmp <a> <b> -c 合并".to_string(),));
    }
    if compact {
        if names.len() < 2 {
            return Err(PocError::Usage("合并需要至少两个 Compose 名".to_string()));
        }
        return Err(PocError::Msg(
            "Compose 合并需要行级 graft（M2/M3），尚未实现".to_string(),));
    }
    if names.len() != 1 {
        return Err(PocError::Usage(
            "切换需要恰一个 Compose 名（合并用 -c）".to_string(),));
    }
    cmd_cmp_switch(ctx, names[0].clone())
}

fn cmd_cmp_new(ctx: &Ctx, name: String, fork: bool) -> Res<()> {
    validate_compose_name(&name)?;
    let (_, cur) = require_current(ctx)?;
    if ctx.store.compose_get(&name)?.is_some() {
        return Err(PocError::Msg(format!("Compose 已存在：{name}")));
    }
    let st = if fork {
        ComposeState {
            base: cur.base,
            head: cur.head,
            ops: cur.ops.clone(),
        }
    } else {
        ComposeState {
            base: cur.head,
            head: cur.head,
            ops: vec![],
        }
    };
    if ctx.globals.dry_run {
        println!(
            "[dry-run] 将新建 Compose `{name}`（{}）并切换（不落盘）",
            if fork { "复刻整栈" } else { "新历史，base = 当前 head" }
        );
        return Ok(());
    }
    ctx.store.compose_put(&name, &st)?;
    ctx.store.meta_set("current", &name)?;
    println!(
        "已新建 Compose `{name}` 并切换（{}）",
        if fork { "复刻整栈" } else { "新历史" }
    );
    Ok(())
}

fn cmd_cmp_switch(ctx: &Ctx, name: String) -> Res<()> {
    validate_compose_name(&name)?;
    let st = ctx
        .store
        .compose_get(&name)?
        .ok_or_else(|| PocError::NotFound(format!("Compose `{name}`")))?;
    let (cur_name, cur) = require_current(ctx)?;
    if name == cur_name {
        println!("已在 Compose `{name}`");
        return Ok(());
    }
    let cur_head_tree = ctx
        .store
        .get_tree(&cur.head)?
        .ok_or_else(|| PocError::Msg("当前 head 树缺失".into()))?;
    let work = tree::scan(&ctx.root)?;
    if !tree::disk_matches_tree(&work, &cur_head_tree) {
        return Err(PocError::Dirty(
            "先 `poc opt -M` 记录变更后再切换".into(),
        ));
    }
    let target_tree = ctx
        .store
        .get_tree(&st.head)?
        .ok_or_else(|| PocError::Msg("目标 head 树缺失".into()))?;
    tree::materialize(&ctx.root, &target_tree, &ctx.store, &work)?;
    ctx.store.meta_set("current", &name)?;
    println!("已切换到 Compose `{name}`（head {}）", st.head.short());
    Ok(())
}

// ---------- status ----------

pub fn cmd_status(ctx: &Ctx) -> Res<()> {
    let mut pool_rows: Vec<(Hash, u64, String)> = Vec::new();
    for (h, t) in ctx.store.pool_list()? {
        let msg = ctx
            .store
            .get_op(&h)?
            .map(|o| o.msg)
            .unwrap_or_else(|| "<对象缺失>".into());
        pool_rows.push((h, t, msg));
    }
    pool_rows.sort_by_key(|(_, t, _)| *t);
    println!("自由池（{}）:", pool_rows.len());
    if pool_rows.is_empty() {
        println!("  （空）");
    }
    for (h, _, msg) in &pool_rows {
        println!("  {}  \"{msg}\"", h.short());
    }

    if let Some(name) = ctx.store.current_name()? {
        let st = ctx
            .store
            .compose_get(&name)?
            .ok_or_else(|| PocError::Msg(format!("Compose `{name}` 不存在")))?;
        println!("\nCompose: {name}");
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
        if d.is_empty() {
            println!("未记录变更：（无，工作区与 head 一致）");
        } else {
            let hunks = hunk_total(ctx, &head_map, &d, &work);
            println!(
                "未记录变更 —— 整体: +{} −{} 文件；内部: {} 文件（{} hunks）",
                d.added.len(),
                d.removed.len(),
                d.modified.len(),
                hunks
            );
            for p in &d.added {
                println!("  + {p}");
            }
            for p in &d.removed {
                println!("  − {p}");
            }
            for p in &d.modified {
                println!("  M {p}");
            }
        }
    } else {
        println!("\n（无当前 Compose）");
    }

    let step = ctx.store.meta_get("step")?;
    println!("\n步骤: {}", step.unwrap_or_else(|| "无".into()));
    Ok(())
}

// ---------- config ----------

pub fn cmd_config(ctx: &Ctx, unset: bool, key: Option<String>, value: Option<String>) -> Res<()> {
    match (unset, key, value) {
        (false, None, _) => {
            println!("生效配置：");
            for k in [config::USER_NAME, config::USER_EMAIL] {
                match config::resolve_key(&ctx.store, k)? {
                    Some((v, s)) => println!("  {k} = {v}   （{}）", s.label()),
                    None => println!("  {k} = （未设置）"),
                }
            }
            println!(
                "  project.name = {}",
                ctx.store.meta_get("project.name")?.unwrap_or_default()
            );
            Ok(())
        }
        (true, Some(k), None) => {
            if !config::valid_config_key(&k) {
                return Err(PocError::Usage(format!("不支持的配置键：{k}")));
            }
            ctx.store.meta_del(&k)?;
            println!("已删除 {k}");
            Ok(())
        }
        (false, Some(k), Some(v)) => {
            if !config::valid_config_key(&k) {
                return Err(PocError::Usage(format!("不支持的配置键：{k}")));
            }
            let v = v.trim().to_string();
            ctx.store.meta_set(&k, &v)?;
            println!("已写入 {k} = {v}（本仓库）");
            Ok(())
        }
        _ => Err(PocError::Usage(
            "用法：poc config | poc config <key> <value> | poc config --unset <key>".to_string(),)),
    }
}

// ---------- gc ----------

pub fn cmd_gc(ctx: &Ctx) -> Res<()> {
    let (b, t, o) = ctx.store.gc_run()?;
    println!(
        "gc 完成：清理 blob {b} / tree {t} / op {o}"
    );
    Ok(())
}
