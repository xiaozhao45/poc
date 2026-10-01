//! P.O.C. — Project · Operation · Compose
//! 用户态本地版本控制工具核心库：全部业务逻辑都在这里，poc / dpoc / cpoc / fpoc 四个二进制共用。

pub mod cli;
pub mod cmds;
pub mod config;
pub mod db;
pub mod dcmds;
pub mod diff;
pub mod err;
pub mod hash;
pub mod object;
pub mod render;
pub mod tree;
pub mod ui;

/// 对象类型标签：id = BLAKE3(tag ‖ 规范载荷)
pub const TAG_BLOB: u8 = 0x01;
pub const TAG_TREE: u8 = 0x02;
pub const TAG_OP: u8 = 0x03;
