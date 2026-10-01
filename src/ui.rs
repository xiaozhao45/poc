//! 交互原语：TTY 检测、编辑器调用（$POC_EDITOR → $VISUAL → $EDITOR → 平台缺省）。

use std::io::IsTerminal;
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
