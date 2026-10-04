//! 展示层：时间人性化、显示宽度对齐、块式渲染（compose / stack / op 详情 /
//! 自由池行 / 待办步骤节）。
//! 结构定式：小节标题顶格，正文缩进 2 空格，次级信息缩进 4 空格；对齐一律基于
//! 内容（显示宽度），不做任何终端宽度探测；箭头一律 ASCII `->`。

use std::path::Path;

use rust_i18n::t;

use crate::hash::Hash;
use crate::object::Op;
use crate::step::Step;
use crate::theme::{Token, paint};

pub fn fmt_time(ms: u64) -> String {
    match jiff::Timestamp::from_millisecond(ms as i64) {
        Ok(t) => {
            let tz = jiff::tz::TimeZone::system();
            t.to_zoned(tz).strftime("%Y-%m-%d %H:%M").to_string()
        }
        Err(_) => format!("<ts {ms}>"),
    }
}

/// 显示宽度（CJK 与全角记 2 列）。只用于内容对齐，与终端尺寸无关。
pub fn disp_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = matches!(u,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6);
    if wide { 2 } else { 1 }
}

/// 按显示宽度右侧补空格。
pub fn pad_disp(s: &str, w: usize) -> String {
    let dw = disp_width(s);
    if dw >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - dw))
    }
}

/// `compose <name>(current)` 标题行。
pub fn compose_line(name: &str, current: bool) -> String {
    let cur = if current {
        t!("show.current").to_string()
    } else {
        String::new()
    };
    paint(Token::Header, format!("compose {name}{cur}"))
}

/// head 行：不变或移动（含物化说明）。
pub fn head_line(old: &Hash, new: &Hash) -> String {
    if old == new {
        t!("stack.head_unchanged", id = paint(Token::Id, old.short())).to_string()
    } else {
        t!(
            "stack.head_moved",
            from = paint(Token::Id, old.short()),
            to = paint(Token::Id, new.short())
        )
        .to_string()
    }
}

/// pool 行：入池清单或"无对象入池"。
pub fn pooled_line(pooled: &[Hash]) -> String {
    if pooled.is_empty() {
        return t!("stack.nothing_pooled").to_string();
    }
    let ids: Vec<String> = pooled
        .iter()
        .map(|h| paint(Token::Id, h.short()).to_string())
        .collect();
    t!("stack.pooled", ids = ids.join(", ")).to_string()
}

/// 变更计数摘要（着色，零类省略）。
pub fn counts_summary(added: usize, removed: usize, modified: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if added > 0 {
        parts.push(paint(
            Token::Added,
            t!("status.n_added", n = added).to_string(),
        ));
    }
    if removed > 0 {
        parts.push(paint(
            Token::Removed,
            t!("status.n_deleted", n = removed).to_string(),
        ));
    }
    if modified > 0 {
        parts.push(paint(
            Token::Updated,
            t!("status.n_updated", n = modified).to_string(),
        ));
    }
    parts.join(", ")
}

fn base_head_rows(base: &Hash, head: &Hash) -> String {
    format!(
        "  base:  {}\n  head:  {}\n",
        paint(Token::Id, base.short()),
        paint(Token::Id, head.short())
    )
}

/// `poc show` 默认形态：compose 头 + 独立的 stack 小节（与 compose 同级顶格）。
pub fn show_stack(name: &str, base: &Hash, head: &Hash, ops: &[(Hash, Op)]) -> String {
    let mut s = String::new();
    s.push_str(&compose_line(name, true));
    s.push('\n');
    s.push_str(&base_head_rows(base, head));
    s.push('\n');
    if ops.is_empty() {
        s.push_str(&format!("{}\n", t!("show.stack_empty")));
    } else {
        s.push_str(&format!(
            "{}\n",
            paint(Token::Header, t!("show.stack_header", n = ops.len()))
        ));
        for (h, op) in ops.iter().rev() {
            s.push_str(&op_entry(h, op));
            s.push('\n');
        }
    }
    s
}

/// `poc show -c`：每个 compose 一个块，`note` 描述其栈况。
pub fn compose_block(name: &str, base: &Hash, head: &Hash, current: bool, note: &str) -> String {
    format!(
        "{}\n{}  {}\n",
        compose_line(name, current),
        base_head_rows(base, head),
        note
    )
}

/// 单个 operation 条目：id + 消息一行，作者/时间缩进一行（dim）。
pub fn op_entry(id: &Hash, op: &Op) -> String {
    format!(
        "  {}  \"{}\"\n    {}",
        paint(Token::Id, id.short()),
        op.msg,
        paint(
            Token::Dim,
            format!("{} · {}", op.author.display(), fmt_time(op.time_ms))
        )
    )
}

/// `poc show <id>`：operation 详情（元数据缩进对齐 + 原样 diff）。
pub fn op_detail(h: &Hash, op: &Op, diff: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        paint(Token::Header, "operation"),
        paint(Token::Id, h.hex())
    ));
    let rows = [
        (t!("show.k_message").to_string(), op.msg.clone()),
        (t!("show.k_author").to_string(), op.author.display()),
        (t!("show.k_time").to_string(), fmt_time(op.time_ms)),
        (t!("show.k_pre").to_string(), op.pre.hex()),
        (t!("show.k_post").to_string(), op.post.hex()),
    ];
    let w = rows.iter().map(|(k, _)| disp_width(k)).max().unwrap_or(0);
    for (k, v) in &rows {
        out.push_str(&format!(
            "  {}  {}\n",
            paint(Token::Dim, pad_disp(&format!("{k}:"), w + 1)),
            v
        ));
    }
    out.push('\n');
    out.push_str(diff);
    out
}

/// 待办步骤节：`poc -s` 与 `poc status` 共用同一渲染器。
pub fn step_section(s: &Step, root: &Path) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}: {}\n",
        paint(Token::Header, t!("step.pending_label")),
        crate::step::kind_label(s.kind)
    ));
    let swap_root = crate::step::swap_root(root);
    let mut rows: Vec<(String, String)> =
        vec![(t!("step.operations").to_string(), s.ids.join(" "))];
    if !s.message.is_empty() {
        rows.push((t!("step.message").to_string(), s.message.clone()));
    }
    if !s.result_name.is_empty() {
        rows.push((t!("step.result").to_string(), s.result_name.clone()));
    }
    let w = rows.iter().map(|(k, _)| disp_width(k)).max().unwrap_or(0);
    for (k, v) in &rows {
        out.push_str(&format!("  {} {}\n", paint(Token::Dim, pad_disp(k, w)), v));
    }
    for (_, path, fname) in &s.swaps {
        out.push_str(&format!(
            "  {} -> {}\n",
            path,
            swap_root.join(fname).display()
        ));
    }
    out.push_str(&format!("  {}\n", paint(Token::Dim, t!("step.swap_hint"))));
    out
}
