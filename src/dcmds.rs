//! `dpoc`：危险操作隔离程序（只进行危险操作及其安全网）。
//! 三层鉴权（独立二进制 / dp.enabled / TTY 全 id 确认）+ 动词级许可 + attic 两段式销毁 + 审计。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anstream::println;
use rust_i18n::t;

use crate::cli::Globals;
use crate::theme::{paint, Token};
use crate::cmds::{resolve_op, Ctx};
use crate::err::{PocError, Res};
use crate::hash::Hash;
use crate::log::{Event, EVT_DESTROY, EVT_PURGE, EVT_REMOVE, EVT_RESTORE, POOL_SCOPE};
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
            t!("dp.not_enabled").to_string(),));
    }
    Ok(())
}

fn require_tty() -> Res<()> {
    if !crate::ui::stdout_is_tty() {
        return Err(PocError::Msg(
            t!("dp.not_tty").to_string(),));
    }
    Ok(())
}

fn confirm_typed(expected: &str) -> Res<()> {
    println!("{}", t!("dp.confirm_prompt", expected = expected));
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if line.trim() == expected {
        Ok(())
    } else {
        Err(PocError::Msg(t!("dp.confirm_mismatch").to_string()))
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
            println!(
                "{}",
                rust_i18n::t!("gc.done_deep", blobs = b, trees = t, ops = o)
            );
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
        other => Err(PocError::Usage(t!("dp.unknown_verb", verb = other).to_string())),
    }
}

fn print_help() {
    println!(
        "dpoc — P.O.C. danger operations (only danger verbs and their safety nets)
usage:
  dpoc enable | disable
  dpoc opt-destroy <ids…>           # destroy ops -> attic (stack members need compose-remove first)
  dpoc compose-remove <names…>      # remove composes; members sink into the free pool
  dpoc compose-destroy <names…>     # remove and destroy members -> attic
  dpoc restore <ids…>               # recover from the attic
  dpoc attic-purge <ids…>           # delete for real (irreversible)
  dpoc gc | verify | audit
permission: $POC_CONFIG_DIR/dpoc.conf or ~/.config/poc/dpoc.conf, one `verb = allow|deny` per line; destroy verbs default to deny."
    );
}

fn enable() -> Res<()> {
    require_tty()?;
    println!("{}", t!("dp.enable_prompt"));
    confirm_typed("ENABLE")?;
    let ctx = Ctx::open(Globals::default())?;
    ctx.store.meta_set("dp.enabled", "1")?;
    ctx.store.audit_append("enable", &[], now_ms())?;
    println!("{}", t!("dp.enabled"));
    Ok(())
}

fn disable() -> Res<()> {
    let ctx = Ctx::open(Globals::default())?;
    ctx.store.meta_set("dp.enabled", "0")?;
    ctx.store.audit_append("disable", &[], now_ms())?;
    println!("{}", t!("dp.disabled"));
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
        return Err(PocError::Usage(t!("dp.usage_destroy").to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("opt-destroy") {
        return Err(PocError::Msg(t!("dp.verb_denied", verb = "opt-destroy").to_string()));
    }
    let pool: HashSet<Hash> = ctx.store.pool_list()?.into_iter().map(|(h, _)| h).collect();
    for s in ids {
        let (h, op) = resolve_op(ctx, s)?;
        if in_attic(ctx, &h)? {
            println!("{}", t!("dp.already_attic", id = paint(Token::Id, h.short())));
            continue;
        }
        if !pool.contains(&h) {
            return Err(PocError::Msg(
                t!("dp.destroy_on_stack", id = h.short()).to_string(),
            ));
        }
        confirm_typed(&format!("DESTROY {}", h.hex()))?;
        let ev = Event::new(
            EVT_DESTROY,
            POOL_SCOPE,
            Some(h),
            "",
            "destroyed (moved to attic)",
            &op.msg,
            now_ms(),
        );
        ctx.store.destroy_from_pool(&h, "自由池", "opt-destroy", &ev)?;
        println!("{}", t!("dp.destroyed", id = paint(Token::Id, h.short()), msg = op.msg));
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
            println!("{}", t!("dp.current_switched", name = n));
            // 不自动物化（可能覆盖未记录内容）；不一致时明确告知
            if let Some(st) = ctx.store.compose_get(n)? {
                if let Some(t) = ctx.store.get_tree(&st.head)? {
                    let work = crate::tree::scan(&ctx.root)?;
                    if !crate::tree::disk_matches_tree(&work, &t) {
                        println!("{}", t!("dp.current_dirty", name = n));
                    }
                }
            }
        }
        None => {
            ctx.store.meta_del("current")?;
            println!("{}", t!("dp.no_composes"));
        }
    }
    Ok(())
}

fn compose_remove(ctx: &Ctx, names: &[String]) -> Res<()> {
    if names.is_empty() {
        return Err(PocError::Usage(t!("dp.usage_compose").to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("compose-remove") {
        return Err(PocError::Msg(t!("dp.verb_denied", verb = "compose-remove").to_string()));
    }
    for name in names {
        let st = ctx
            .store
            .compose_get(name)?
            .ok_or_else(|| PocError::NotFound(format!("Compose `{name}`")))?;
        let shorts: Vec<String> = st.ops.iter().map(|h| h.short()).collect();
        let ev = Event::new(
            EVT_REMOVE,
            name,
            None,
            "",
            "compose removed; members pooled to the free pool",
            &shorts.join(" "),
            now_ms(),
        );
        ctx.store
            .compose_dispose(name, &st.ops, false, "", "compose-remove", &ev)?;
        println!("{}", t!("dp.removed", name = name));
        switch_current_away(ctx, name)?;
    }
    Ok(())
}

fn compose_destroy(ctx: &Ctx, names: &[String]) -> Res<()> {
    if names.is_empty() {
        return Err(PocError::Usage(t!("dp.usage_compose").to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("compose-destroy") {
        return Err(PocError::Msg(t!("dp.verb_denied", verb = "compose-destroy").to_string()));
    }
    for name in names {
        let st = ctx
            .store
            .compose_get(name)?
            .ok_or_else(|| PocError::NotFound(format!("Compose `{name}`")))?;
        confirm_typed(&format!("DESTROY {name}"))?;
        let shorts: Vec<String> = st.ops.iter().map(|h| h.short()).collect();
        let ev = Event::new(
            EVT_DESTROY,
            name,
            None,
            "",
            "compose destroyed; members moved to the attic",
            &shorts.join(" "),
            now_ms(),
        );
        ctx.store.compose_dispose(
            name,
            &st.ops,
            true,
            &format!("compose:{name}"),
            "compose-destroy",
            &ev,
        )?;
        println!("{}", t!("dp.compose_destroyed", name = name));
        switch_current_away(ctx, name)?;
    }
    Ok(())
}

fn restore(ctx: &Ctx, ids: &[String]) -> Res<()> {
    if ids.is_empty() {
        return Err(PocError::Usage(t!("dp.usage_restore").to_string()));
    }
    for s in ids {
        let (h, op) = resolve_op(ctx, s)?;
        if !in_attic(ctx, &h)? {
            println!("{}", t!("dp.not_in_attic", id = h.short()));
            continue;
        }
        let ev = Event::new(EVT_RESTORE, POOL_SCOPE, Some(h), "", "restored from attic", "", now_ms());
        ctx.store.restore_to_pool(&h, &ev)?;
        println!("{}", t!("dp.restored", id = paint(Token::Id, h.short()), msg = op.msg));
    }
    Ok(())
}

fn attic_purge(ctx: &Ctx, ids: &[String]) -> Res<()> {
    if ids.is_empty() {
        return Err(PocError::Usage(t!("dp.usage_purge").to_string()));
    }
    require_enabled(ctx)?;
    require_tty()?;
    if !verb_allowed("attic-purge") {
        return Err(PocError::Msg(t!("dp.verb_denied", verb = "attic-purge").to_string()));
    }
    for s in ids {
        let (h, _) = resolve_op(ctx, s)?;
        if !in_attic(ctx, &h)? {
            return Err(PocError::Msg(
                t!("dp.purge_not_attic", id = h.short()).to_string(),
            ));
        }
        confirm_typed(&format!("PURGE {}", h.hex()))?;
        let ev = Event::new(
            EVT_PURGE,
            POOL_SCOPE,
            Some(h),
            "",
            "purged (this log line remains)",
            "",
            now_ms(),
        );
        ctx.store.purge_op_row(&h, &ev)?;
        println!("{}", t!("dp.purged", id = paint(Token::Id, h.short())));
    }
    Ok(())
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
                .ok_or_else(|| PocError::Msg(t!("dp.verify_missing_tree", id = h.short()).to_string()))?;
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
                .ok_or_else(|| PocError::Msg(t!("dp.verify_compose_tree", name = name).to_string()))?;
        }
        for h in &st.ops {
            ctx.store
                .get_op(h)?
                .ok_or_else(|| PocError::Msg(t!("dp.verify_compose_op", name = name).to_string()))?;
        }
    }
    let seq: u64 = ctx
        .store
        .meta_get("audit.seq")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if ctx.store.audit_list()?.len() as u64 != seq {
        return Err(PocError::Msg(t!("dp.verify_audit_gap").to_string()));
    }
    let lseq: u64 = ctx
        .store
        .meta_get("log.seq")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if ctx.store.log_list()?.len() as u64 != lseq {
        return Err(PocError::Msg(t!("dp.verify_log_gap").to_string()));
    }
    println!(
        "{}",
        t!("dp.verify_ok", n = ops.len())
    );
    Ok(())
}
