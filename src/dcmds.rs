//! `dpoc`：危险操作隔离程序（只进行危险操作及其安全网）。
//! 三层鉴权（独立二进制 / dp.enabled / TTY 全 id 确认）+ 动词级许可 + attic 两段式销毁 + 审计。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::Globals;
use crate::cmds::{resolve_op, Ctx};
use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::render;

/// 销毁类动词：dpoc.conf 默认 deny，未列出即拒绝。
const DESTROY_VERBS: &[&str] = &[
    "opt-destroy",
    "compose-destroy",
    "attic-purge",
    "compose-remove",
];

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn conf_path() -> PathBuf {
    if let Ok(p) = std::env::var("POC_CONFIG_DIR") {
        return Path::new(&p).join("dpoc.conf");
    }
    dirs::config_dir()
        .map(|d| d.join("poc").join("dpoc.conf"))
        .unwrap_or_else(|| PathBuf::from("dpoc.conf"))
}

fn verb_allowed(verb: &str) -> bool {
    if !DESTROY_VERBS.contains(&verb) {
        return true; // enable/disable/restore/verify/audit/gc 默认 allow
    }
    let text = match std::fs::read_to_string(conf_path()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == verb {
                return v.trim().eq_ignore_ascii_case("allow");
            }
        }
    }
    false
}

fn require_enabled(ctx: &Ctx) -> Res<()> {
    if ctx.store.meta_get("dp.enabled")?.as_deref() != Some("1") {
        return Err(PocError::Msg(
            "危险模式未开启：先 `dpoc enable`（交互确认）".to_string(),));
    }
    Ok(())
}

fn require_tty() -> Res<()> {
    if !crate::ui::stdout_is_tty() {
        return Err(PocError::Msg(
            "dpoc 危险动词必须在交互终端执行（拒绝非 TTY）".to_string(),));
    }
    Ok(())
}

fn confirm_typed(expected: &str) -> Res<()> {
    println!("确认：请完整输入 `{expected}`");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if line.trim() == expected {
        Ok(())
    } else {
        Err(PocError::Msg("确认不匹配，已取消".to_string()))
    }
}

pub fn run(args: &[String]) -> Res<()> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = args[1.min(args.len())..]
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();
    match verb {
        "enable" => enable(),
        "disable" => disable(),
        "audit" => {
            let ctx = Ctx::open(Globals::default())?;
            audit(&ctx)
        }
        "verify" => {
            let ctx = Ctx::open(Globals::default())?;
            verify(&ctx)
        }
        "gc" => {
            let ctx = Ctx::open(Globals::default())?;
            let (b, t, o) = ctx.store.gc_run()?;
            println!("gc --deep 完成：清理 blob {b} / tree {t} / op {o}");
            Ok(())
        }
        "opt-destroy" => {
            let ctx = Ctx::open(Globals::default())?;
            opt_destroy(&ctx, &rest)
        }
        "compose-remove" => {
            let ctx = Ctx::open(Globals::default())?;
            compose_remove(&ctx, &rest)
        }
        "compose-destroy" => {
            let ctx = Ctx::open(Globals::default())?;
            compose_destroy(&ctx, &rest)
        }
        "restore" => {
            let ctx = Ctx::open(Globals::default())?;
            restore(&ctx, &rest)
        }
        "attic-purge" => {
            let ctx = Ctx::open(Globals::default())?;
            attic_purge(&ctx, &rest)
        }
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => Err(PocError::Usage(format!("未知 dpoc 动词：{other}"))),
    }
}

fn print_help() {
    println!(
        "dpoc — P.O.C. 危险操作程序（只进行危险操作及其安全网）
用法：
  dpoc enable | disable
  dpoc opt-destroy <ids…>           # 销毁 opt → attic（栈内 opt 需先 compose-remove 或等待 M3）
  dpoc compose-remove <names…>      # 移除 Compose，成员沉淀自由池
  dpoc compose-destroy <names…>     # 移除并连成员销毁 → attic
  dpoc restore <ids…>               # 从 attic 找回
  dpoc attic-purge <ids…>           # 真删除（不可逆）
  dpoc gc | verify | audit
许可：$POC_CONFIG_DIR/dpoc.conf 或 ~/.config/poc/dpoc.conf，一行 `verb = allow|deny`；销毁类默认 deny。"
    );
}

fn enable() -> Res<()> {
    require_tty()?;
    println!(
        "dpoc enable：允许物理销毁对象。销毁默认进入 attic（可 restore），attic-purge 才不可逆。"
    );
    confirm_typed("ENABLE")?;
    let ctx = Ctx::open(Globals::default())?;
    ctx.store.meta_set("dp.enabled", "1")?;
    ctx.store.audit_append("enable", &[], now_ms())?;
    println!("危险模式已开启（本项目）。");
    Ok(())
}

fn disable() -> Res<()> {
    let ctx = Ctx::open(Globals::default())?;
    ctx.store.meta_set("dp.enabled", "0")?;
    ctx.store.audit_append("disable", &[], now_ms())?;
    println!("危险模式已关闭。");
    Ok(())
}

fn in_attic(ctx: &Ctx, h: &Hash) -> Res<bool> {
    Ok(ctx
        .store
        .attic_list()?
        .iter()
        .any(|(a, _, _)| a == h))
}

fn opt_destroy(ctx: &Ctx, ids: &[String]) -> Res<()> {
    if ids.is_empty() {
        return Err(PocError::Usage("dpoc opt-destroy <ids…>".to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("opt-destroy") {
        return Err(PocError::Msg("许可：opt-destroy 被 dpoc.conf 拒绝".to_string()));
    }
    let pool: HashSet<Hash> = ctx.store.pool_list()?.into_iter().map(|(h, _)| h).collect();
    for s in ids {
        let (h, op) = resolve_op(ctx, s)?;
        if in_attic(ctx, &h)? {
            println!("已在 attic，跳过：{}", h.short());
            continue;
        }
        if !pool.contains(&h) {
            return Err(PocError::Msg(format!(
                "opt {} 在某个 Compose 栈内：栈内销毁需要重建上层（M3 graft）。可先 `dpoc compose-remove`。",
                h.short()
            )));
        }
        confirm_typed(&format!("DESTROY {}", h.hex()))?;
        ctx.store.pool_remove(&h)?;
        ctx.store.attic_add(&h, "自由池", now_ms())?;
        ctx.store.audit_append("opt-destroy", &[h], now_ms())?;
        println!("已移入 attic：{}  \"{}\"", h.short(), op.msg);
    }
    Ok(())
}

fn switch_current_away(ctx: &Ctx, removed: &str) -> Res<()> {
    if ctx.store.current_name()?.as_deref() != Some(removed) {
        return Ok(());
    }
    match ctx.store.compose_names()?.first() {
        Some(n) => {
            ctx.store.meta_set("current", n)?;
            println!("当前 Compose 切换为 `{n}`");
        }
        None => {
            ctx.store.meta_del("current")?;
            println!("项目已无 Compose；用 `poc cmp -N` 新建");
        }
    }
    Ok(())
}

fn compose_remove(ctx: &Ctx, names: &[String]) -> Res<()> {
    if names.is_empty() {
        return Err(PocError::Usage("dpoc compose-remove <names…>".to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("compose-remove") {
        return Err(PocError::Msg("许可：compose-remove 被 dpoc.conf 拒绝".to_string()));
    }
    for name in names {
        let st = ctx
            .store
            .compose_get(name)?
            .ok_or_else(|| PocError::NotFound(format!("Compose `{name}`")))?;
        for h in &st.ops {
            ctx.store.pool_add(h, now_ms())?;
        }
        ctx.store.compose_delete(name)?;
        ctx.store.audit_append("compose-remove", &st.ops, now_ms())?;
        println!(
            "已移除 Compose `{name}`，{} 个成员沉淀自由池",
            st.ops.len()
        );
        switch_current_away(ctx, name)?;
    }
    Ok(())
}

fn compose_destroy(ctx: &Ctx, names: &[String]) -> Res<()> {
    if names.is_empty() {
        return Err(PocError::Usage("dpoc compose-destroy <names…>".to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("compose-destroy") {
        return Err(PocError::Msg("许可：compose-destroy 被 dpoc.conf 拒绝".to_string()));
    }
    for name in names {
        let st = ctx
            .store
            .compose_get(name)?
            .ok_or_else(|| PocError::NotFound(format!("Compose `{name}`")))?;
        confirm_typed(&format!("DESTROY {name}"))?;
        for h in &st.ops {
            ctx.store.attic_add(h, &format!("compose:{name}"), now_ms())?;
        }
        ctx.store.compose_delete(name)?;
        ctx.store.audit_append("compose-destroy", &st.ops, now_ms())?;
        println!(
            "已销毁 Compose `{name}`（{} 个成员进 attic）",
            st.ops.len()
        );
        switch_current_away(ctx, name)?;
    }
    Ok(())
}

fn restore(ctx: &Ctx, ids: &[String]) -> Res<()> {
    if ids.is_empty() {
        return Err(PocError::Usage("dpoc restore <ids…>".to_string()));
    }
    for s in ids {
        let (h, op) = resolve_op(ctx, s)?;
        if !in_attic(ctx, &h)? {
            println!("不在 attic：{}", h.short());
            continue;
        }
        ctx.store.attic_remove(&h)?;
        ctx.store.pool_add(&h, now_ms())?;
        ctx.store.audit_append("restore", &[h], now_ms())?;
        println!(
            "已从 attic 恢复到自由池：{}  \"{}\"",
            h.short(),
            op.msg
        );
    }
    Ok(())
}

fn attic_purge(ctx: &Ctx, ids: &[String]) -> Res<()> {
    if ids.is_empty() {
        return Err(PocError::Usage("dpoc attic-purge <ids…>".to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("attic-purge") {
        return Err(PocError::Msg("许可：attic-purge 被 dpoc.conf 拒绝".to_string()));
    }
    for s in ids {
        let (h, _) = resolve_op(ctx, s)?;
        if !in_attic(ctx, &h)? {
            return Err(PocError::Msg(format!(
                "{} 不在 attic（只允许 purge attic 内的对象）",
                h.short()
            )));
        }
        confirm_typed(&format!("PURGE {}", h.hex()))?;
        purge_op(ctx, &h)?;
        ctx.store.attic_remove(&h)?;
        ctx.store.audit_append("attic-purge", &[h], now_ms())?;
        println!("已真删除：{}", h.short());
    }
    Ok(())
}

/// 删除 op 行；其 pre/post 树若无其他引用则一并删除（blobs 交给 gc）。
fn purge_op(ctx: &Ctx, h: &Hash) -> Res<()> {
    let op = ctx
        .store
        .get_op(h)?
        .ok_or_else(|| PocError::Msg("对象缺失".into()))?;
    ctx.store.delete_op_row(h)?;
    for t in [op.pre, op.post] {
        if !tree_referenced(ctx, &t)? {
            ctx.store.delete_tree_row(&t)?;
        }
    }
    Ok(())
}

fn tree_referenced(ctx: &Ctx, t: &Hash) -> Res<bool> {
    for (_, op) in ctx.store.list_ops()? {
        if op.pre == *t || op.post == *t {
            return Ok(true);
        }
    }
    for name in ctx.store.compose_names()? {
        if let Some(st) = ctx.store.compose_get(&name)? {
            if st.base == *t || st.head == *t {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn audit(ctx: &Ctx) -> Res<()> {
    for (seq, verb, ids, time) in ctx.store.audit_list()? {
        let ids: Vec<String> = ids.iter().map(|h| h.short()).collect();
        let ids = if ids.is_empty() {
            "—".to_string()
        } else {
            ids.join(",")
        };
        println!("#{seq}  {verb}  {ids}  {}", render::fmt_time(time));
    }
    Ok(())
}

fn verify(ctx: &Ctx) -> Res<()> {
    let ops = ctx.store.list_ops()?;
    for (h, op) in &ops {
        if crate::db::op_id(op) != *h {
            return Err(PocError::Msg(format!("op 哈希不符：{}", h.short())));
        }
        for t in [op.pre, op.post] {
            ctx.store
                .get_tree(&t)?
                .ok_or_else(|| PocError::Msg(format!("op {} 引用的树缺失", h.short())))?;
        }
    }
    for name in ctx.store.compose_names()? {
        let st = ctx
            .store
            .compose_get(&name)?
            .ok_or_else(|| PocError::Msg("数据不一致".into()))?;
        for t in [st.base, st.head] {
            ctx.store
                .get_tree(&t)?
                .ok_or_else(|| PocError::Msg(format!("compose {name} 引用的树缺失")))?;
        }
        for h in &st.ops {
            ctx.store
                .get_op(h)?
                .ok_or_else(|| PocError::Msg(format!("compose {name} 引用的 op 缺失")))?;
        }
    }
    let seq: u64 = ctx
        .store
        .meta_get("audit.seq")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if ctx.store.audit_list()?.len() as u64 != seq {
        return Err(PocError::Msg("审计日志不连续".to_string()));
    }
    println!("verify 通过：ops {} 条，全部引用与审计连续性正常", ops.len());
    Ok(())
}
