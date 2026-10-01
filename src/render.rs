//! 展示层：时间人性化、ASCII 栈图。

use crate::hash::Hash;
use crate::object::Op;

pub fn fmt_time(ms: u64) -> String {
    match jiff::Timestamp::from_millisecond(ms as i64) {
        Ok(t) => {
            let tz = jiff::tz::TimeZone::system();
            t.to_zoned(tz).strftime("%Y-%m-%d %H:%M").to_string()
        }
        Err(_) => format!("<ts {ms}>"),
    }
}

pub fn op_line(id: &Hash, op: &Op) -> String {
    format!(
        "{}  \"{}\"  {}  {}",
        id.short(),
        op.msg,
        op.author.display(),
        fmt_time(op.time_ms)
    )
}

/// ASCII 栈图：栈顶在上，自顶向下画到 base。
pub fn stack_ascii(name: &str, base: &Hash, head: &Hash, ops: &[(Hash, Op)]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Compose: {name}    base {}    head {}\n",
        base.short(),
        head.short()
    ));
    s.push('\n');
    if ops.is_empty() {
        s.push_str("  （空栈：head == base）\n");
        return s;
    }
    let n = ops.len();
    for (i, (id, op)) in ops.iter().enumerate().rev() {
        let marker = if i + 1 == n { "top " } else { "    " };
        s.push_str(&format!("  {marker}-- {}\n", op_line(id, op)));
        s.push_str("         |\n");
    }
    s.push_str(&format!("  base -- {}\n", base.short()));
    s
}
