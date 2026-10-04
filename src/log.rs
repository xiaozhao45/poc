//! 追加式操作日志（DESIGN v0.6 §5.7 / §3.3 log 表）。
//! 按栈记录一切改变栈/池/opt 归属的事件；只增不改不删（删栈不删史）。
//! 事件行由 db.rs 的复合写方法与业务变更**同一写事务**落盘，崩溃安全同源。

use crate::err::Res;
use crate::hash::Hash;
use crate::object::{Cursor, put_str, put_u64};
use rust_i18n::t;

pub const EVT_INIT: u8 = 1; // proj 初始化（建立 main）
pub const EVT_CMP_NEW: u8 = 2; // cmp -N 新建（含 --fork [op-id]）
pub const EVT_PUSH: u8 = 3; // opt -M 新铸入栈
pub const EVT_AMEND: u8 = 4; // opt --amend（原栈顶入池）
pub const EVT_COMPACT: u8 = 5; // opt -c 压缩（原 opts 入池）
pub const EVT_POP: u8 = 6; // opt -f 弹出入池
pub const EVT_MERGE: u8 = 7; // cmp -c 合并（M3）
pub const EVT_REMOVE: u8 = 8; // dpoc compose-remove（成员入池）
pub const EVT_DESTROY: u8 = 9; // dpoc opt-destroy / compose-destroy（成员入 attic）
pub const EVT_RESTORE: u8 = 10; // dpoc restore（attic → 池）
pub const EVT_PURGE: u8 = 11; // dpoc attic-purge（真删除；本行保留）

/// 非栈作用域事件（自由池内的销毁/找回/清除）统一挂在此名下。
pub const POOL_SCOPE: &str = "(池)";

/// 事件种类标签（展示层本地化；持久化的事件 msg/detail 不翻译）。
pub fn kind_label(kind: u8) -> String {
    match kind {
        EVT_INIT => t!("log.kind_init").to_string(),
        EVT_CMP_NEW => t!("log.kind_new_stack").to_string(),
        EVT_PUSH => t!("log.kind_push").to_string(),
        EVT_AMEND => t!("log.kind_amend").to_string(),
        EVT_COMPACT => t!("log.kind_compact").to_string(),
        EVT_POP => t!("log.kind_pop").to_string(),
        EVT_MERGE => t!("log.kind_merge").to_string(),
        EVT_REMOVE => t!("log.kind_remove").to_string(),
        EVT_DESTROY => t!("log.kind_destroy").to_string(),
        EVT_RESTORE => t!("log.kind_restore").to_string(),
        EVT_PURGE => t!("log.kind_purge").to_string(),
        _ => "??".into(),
    }
}

/// 一条日志事件。op/author/msg 是**快照**：op 对象被 purge 后本行仍自足可读。
#[derive(Debug, Clone)]
pub struct Event {
    pub time_ms: u64,
    pub kind: u8,
    pub compose: String,
    pub op: Option<Hash>,
    pub author: String,
    pub msg: String,
    pub detail: String,
}

impl Event {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: u8,
        compose: &str,
        op: Option<Hash>,
        author: &str,
        msg: &str,
        detail: &str,
        time_ms: u64,
    ) -> Self {
        Event {
            time_ms,
            kind,
            compose: compose.to_string(),
            op,
            author: author.to_string(),
            msg: msg.to_string(),
            detail: detail.to_string(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        put_u64(&mut v, self.time_ms);
        v.push(self.kind);
        put_str(&mut v, &self.compose);
        match self.op {
            Some(h) => {
                v.push(1);
                v.extend_from_slice(&h.0);
            }
            None => v.push(0),
        }
        put_str(&mut v, &self.author);
        put_str(&mut v, &self.msg);
        put_str(&mut v, &self.detail);
        v
    }

    pub fn decode(b: &[u8]) -> Res<Event> {
        let mut c = Cursor::new(b);
        let time_ms = c.u64()?;
        let kind = c.byte()?;
        let compose = c.str()?;
        let op = if c.byte()? == 1 {
            Some(c.hash()?)
        } else {
            None
        };
        let author = c.str()?;
        let msg = c.str()?;
        let detail = c.str()?;
        c.end()?;
        Ok(Event {
            time_ms,
            kind,
            compose,
            op,
            author,
            msg,
            detail,
        })
    }

    /// 单行渲染（seq 为日志键）。行结构：
    /// `  #seq  time  kind  [id]  [author]  ["msg"]  [— detail]`；
    /// 无 op 的事件不留占位段；kind 列按显示宽度对齐（en 最宽 new-stack = 9）。
    pub fn line(&self, seq: u64) -> String {
        use crate::render::pad_disp;
        use crate::theme::{Token, paint};
        let mut s = format!("  #{:<4}  {}  ", seq, crate::render::fmt_time(self.time_ms));
        s.push_str(&pad_disp(&kind_label(self.kind), 9));
        if let Some(h) = self.op {
            s.push_str(&format!("  {}", paint(Token::Id, h.short())));
        }
        if !self.author.is_empty() {
            s.push_str(&format!("  {}", self.author));
        }
        if !self.msg.is_empty() {
            if self.op.is_some() {
                s.push_str(&format!("  \"{}\"", self.msg));
            } else {
                s.push_str(&format!("  {}", self.msg));
            }
        }
        if !self.detail.is_empty() {
            s.push_str(&format!(" — {}", paint(Token::Dim, &self.detail)));
        }
        s
    }
}
