//! 输出主题：全程序唯一的样式出口（anstyle）。
//! 管线：stdout/stderr 一律经 anstream 宏输出——TTY 保留 ANSI；管道 / NO_COLOR /
//! CLICOLOR=0 自动剥离；pager（minus）对 ANSI 透明（内部按转义感知计算宽度）。
//! 铁律：颜色不承载信息——无色环境下一切输出语义不变。

use anstyle::{AnsiColor, Color, Style};

#[derive(Debug, Clone, Copy)]
pub enum Token {
    /// `error:` 前缀
    Error,
    /// `usage:` 前缀（用法错误，exit 2）
    Usage,
    /// 动词结果行
    Ok,
    /// 短 id / 全 id
    Id,
    /// `+ path` 行与 added 计数
    Added,
    /// `- path` 行与 deleted 计数
    Removed,
    /// `M path` 行与 updated 计数
    Updated,
    /// 小节标题
    Header,
    /// 详情尾巴、来源标注
    Dim,
}

fn fg(c: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(c)))
}

pub fn style(tok: Token) -> Style {
    match tok {
        Token::Error => fg(AnsiColor::Red).bold(),
        Token::Usage => fg(AnsiColor::Yellow).bold(),
        Token::Ok => fg(AnsiColor::Green),
        Token::Id => fg(AnsiColor::Cyan),
        Token::Added => fg(AnsiColor::Green),
        Token::Removed => fg(AnsiColor::Red),
        Token::Updated => fg(AnsiColor::Yellow),
        Token::Header => Style::new().bold(),
        Token::Dim => Style::new().dimmed(),
    }
}

/// 按 token 包裹一段文本（总是嵌入转义码；剥离交给 anstream）。
pub fn paint(tok: Token, text: impl std::fmt::Display) -> String {
    let s = style(tok);
    format!("{}{}{}", s.render(), text, s.render_reset())
}
