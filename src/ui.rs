//! 交互原语：TTY 检测、编辑器调用（$POC_EDITOR → $VISUAL → $EDITOR → 平台缺省）、
//! 内嵌 pager（minus，§5.2：超窗输出交给它，全局 `-p` 禁用）。

use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::err::{PocError, Res};

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// 管道输出（如 `poc show -o | head`）关闭时按 Unix 惯例静默退出而非 panic。
pub fn restore_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// 展示型输出的统一出口：非 TTY（管道）或 `-p` 直印；否则交给内嵌 pager。
/// minus 的 `page_all` 自带两条判定：输出非 TTY 不分页、未超窗不分页。
/// ANSI 转义对 minus 透明（其内部按转义感知计算宽度），主题色可透传进 pager。
pub fn page(no_pager: bool, text: &str) {
    if no_pager || !stdout_is_tty() {
        anstream::print!("{text}");
        let _ = std::io::stdout().flush();
        return;
    }
    let pager = minus::Pager::new();
    if let Err(e) = pager.set_text(text) {
        anstream::eprintln!("pager 初始化失败：{e}");
        anstream::print!("{text}");
        let _ = std::io::stdout().flush();
        return;
    }
    let _ = pager.set_run_no_overflow(true);
    if let Err(e) = minus::page_all(pager) {
        anstream::eprintln!("pager 异常退出：{e}");
    }
}

fn default_editor() -> &'static str {
    if cfg!(windows) {
        "notepad"
    } else {
        "vi"
    }
}

pub fn open_editor(path: &Path) -> Res<()> {
    let ed = std::env::var("POC_EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| default_editor().to_string());
    let status = if cfg!(windows) {
        Command::new(&ed).arg(path).status()?
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(format!("{ed} \"$0\""))
            .arg(path)
            .status()?
    };
    if status.success() {
        Ok(())
    } else {
        Err(PocError::Msg(format!("编辑器退出码非零：{ed}")))
    }
}
