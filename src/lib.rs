//! P.O.C. — Project · Operation · Compose
//! 用户态本地版本控制工具核心库：全部业务逻辑都在这里，poc / dpoc / cpoc / fpoc 四个二进制共用。

// 消息目录：编译期嵌入 locales/*.yml（en 为事实源，缺键回退 en）。
// 必须先于各模块声明展开（t! 宏的文本作用域依赖此顺序）。
rust_i18n::i18n!("locales", fallback = "en");

pub mod cli;
pub mod cmds;
pub mod config;
pub mod db;
pub mod dcmds;
pub mod diff;
pub mod err;
pub mod hash;
pub mod i18n;
pub mod log;
pub mod merge;
pub mod object;
pub mod render;
pub mod step;
pub mod theme;
pub mod tree;
pub mod ui;

#[cfg(test)]
mod properties;

/// 对象类型标签：id = BLAKE3(tag ‖ 规范载荷)
pub const TAG_BLOB: u8 = 0x01;
pub const TAG_TREE: u8 = 0x02;
pub const TAG_OP: u8 = 0x03;
