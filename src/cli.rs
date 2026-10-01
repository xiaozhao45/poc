//! CLI 分派：全局旗标按位置手工预剥离（命令词之前 = 全局，§5.0 消歧规则），
//! 命令词之后交给各命令解析。四个二进制共用本模块。

use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use crate::cmds::{self, Ctx};
use crate::err::{PocError, Res};

#[derive(Debug, Clone, Copy, Default)]
pub struct Globals {
    pub gc: bool,
    pub step: bool,
    pub commit: bool,
    pub cancel: bool,
    pub no_pager: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub yes: bool,
}

fn parse_global(tok: &str, g: &mut Globals) -> bool {
    match tok {
        "-o" | "--gc" => {
            g.gc = true;
            true
        }
        "-s" | "--step" => {
            g.step = true;
            true
        }
        "-c" | "--commit" => {
            g.commit = true;
            true
        }
        "-q" | "--cancel" => {
            g.cancel = true;
            true
        }
        "-p" | "--no-pager" => {
            g.no_pager = true;
            true
        }
        "--quiet" => {
            g.quiet = true;
            true
        }
        "--verbose" => {
            g.verbose = true;
            true
        }
        "--dry-run" => {
            g.dry_run = true;
            true
        }
        "-y" | "--yes" => {
            g.yes = true;
            true
        }
        _ => false,
    }
}

fn split_globals(args: &[String]) -> Res<(Globals, usize)> {
    let mut g = Globals::default();
    let mut i = 0;
    while i < args.len() {
        let tok = args[i].as_str();
        if tok == "--" {
            i += 1;
            break;
        }
        if tok.starts_with('-') && tok.len() > 1 {
            if parse_global(tok, &mut g) {
                i += 1;
            } else {
                return Err(PocError::Usage(format!(
                    "未知全局旗标 `{tok}`（全局旗标必须位于命令词之前；命令局部旗标写在命令词之后）"
                )));
            }
        } else {
            break;
        }
    }
    Ok((g, i))
}

#[derive(Parser)]
#[command(
    name = "poc",
    version,
    about = "P.O.C. — Project·Operation·Compose（安全本地操作）"
)]
struct PocCli {
    #[command(subcommand)]
    cmd: PocCmd,
}

#[derive(clap::Subcommand)]
enum PocCmd {
    /// 创建/初始化 Project
    Proj {
        /// 新建同名目录并初始化
        #[arg(short = 'N', long = "new")]
        new_name: Option<String>,
        /// 在指定目录初始化
        path: Option<String>,
    },
    /// 创建 Operation（-M/-i）或操控既有 opt（ids + -f/-c）
    Opt {
        /// operation ids（缺省 = 创建模式）
        ids: Vec<String>,
        /// 操作消息（缺省打开编辑器）
        #[arg(short = 'M', long = "message")]
        message: Option<String>,
        /// 文件 glob 限定
        #[arg(short = 'i', long = "include")]
        include: Option<String>,
        /// 弹出到自由池
        #[arg(short = 'f', long = "free")]
        free: bool,
        /// 压缩为一个
        #[arg(short = 'c', long = "compact")]
        compact: bool,
        /// 改写栈顶
        #[arg(long = "amend")]
        amend: bool,
    },
    /// 状态总览 / 单 opt 详情
    Show {
        /// 列出所有 Compose
        #[arg(short = 'c', long = "composes")]
        composes: bool,
        /// 列出所有 Operation
        #[arg(short = 'o', long = "operations")]
        operations: bool,
        /// 单 opt id
        id: Option<String>,
    },
    /// Compose 切换 / 新建 / 合并
    Cmp {
        /// compose names（恰一个 = 切换；多个 + -c = 合并）
        names: Vec<String>,
        /// 合并多个 Compose
        #[arg(short = 'c', long = "compact")]
        compact: bool,
        /// 新建 Compose
        #[arg(short = 'N', long = "new")]
        new: Option<String>,
        /// 新建时复刻当前整栈
        #[arg(long = "fork")]
        fork: bool,
    },
    /// 自由池 / 未记录变更 / 步骤
    Status,
    /// 工作区/快照差异
    Diff {
        /// 一个 opt id = 该 opt 的差异；两个 = 两快照差异
        a: Option<String>,
        b: Option<String>,
        /// 只输出统计
        #[arg(long = "stat")]
        stat: bool,
    },
    /// 用户信息等本仓库配置
    Config {
        /// 删除配置键
        #[arg(long)]
        unset: bool,
        key: Option<String>,
        value: Option<String>,
    },
}

fn argv() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn wrap(r: Res<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e @ PocError::Usage(_)) => {
            eprintln!("用法错误：{e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::from(1)
        }
    }
}

pub fn poc_main() -> ExitCode {
    crate::ui::restore_sigpipe();
    wrap(poc_run(&argv()))
}

pub fn dpoc_main() -> ExitCode {
    crate::ui::restore_sigpipe();
    wrap(crate::dcmds::run(&argv()))
}

pub fn cpoc_main() -> ExitCode {
    crate::ui::restore_sigpipe();
    println!("cpoc：链接协作已推迟至 1.x（本二进制为占位）。设计输入见 docs/DESIGN.md §1.5 / §4.5。");
    ExitCode::SUCCESS
}

const DPOC_VERBS: &[&str] = &[
    "enable",
    "disable",
    "opt-destroy",
    "compose-remove",
    "compose-destroy",
    "restore",
    "attic-purge",
    "gc",
    "verify",
    "audit",
];

pub fn fpoc_main() -> ExitCode {
    crate::ui::restore_sigpipe();
    let args = argv();
    let (_, n) = match split_globals(&args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("用法错误：{e}");
            return ExitCode::from(2);
        }
    };
    let first = args.get(n).map(|s| s.as_str()).unwrap_or("");
    if DPOC_VERBS.contains(&first) {
        wrap(crate::dcmds::run(&args))
    } else {
        wrap(poc_run(&args))
    }
}

fn poc_run(args: &[String]) -> Res<()> {
    let (g, n) = split_globals(args)?;
    let rest = &args[n..];

    if rest.is_empty() {
        // 裸全局旗标形态（<any command> 可以为空）
        if g.gc {
            let ctx = Ctx::open(g)?;
            let (b, t, o) = ctx.store.gc_run()?;
            println!("gc 完成：清理 blob {b} / tree {t} / op {o}");
            return Ok(());
        }
        if g.commit || g.cancel {
            return Err(PocError::Msg("无待办步骤".to_string()));
        }
        if g.step {
            let ctx = Ctx::open(g)?;
            match ctx.store.meta_get("step")? {
                Some(s) => println!("步骤: {s}"),
                None => println!("步骤: 无"),
            }
            return Ok(());
        }
        let _ = PocCli::command().print_help();
        return Ok(());
    }

    // proj 不要求已在项目内，先于 Ctx 分派
    if rest[0] == "proj" {
        let cli = PocCli::try_parse_from(std::iter::once("poc".to_string()).chain(rest.iter().cloned()))
            .map_err(|e| PocError::Usage(e.to_string()))?;
        let PocCmd::Proj { new_name, path } = cli.cmd else {
            unreachable!()
        };
        return cmds::cmd_proj(new_name, path);
    }

    let ctx = Ctx::open(g)?;
    if g.gc {
        let (b, t, o) = ctx.store.gc_run()?;
        println!("gc 完成：清理 blob {b} / tree {t} / op {o}");
    }

    let cli = PocCli::try_parse_from(std::iter::once("poc".to_string()).chain(rest.iter().cloned()))
        .map_err(|e| PocError::Usage(e.to_string()))?;
    match cli.cmd {
        PocCmd::Proj { .. } => unreachable!(),
        PocCmd::Opt {
            ids,
            message,
            include,
            free,
            compact,
            amend,
        } => cmds::cmd_opt(
            &ctx,
            cmds::OptArgs {
                ids,
                message,
                include,
                free,
                compact,
                amend,
            },
        ),
        PocCmd::Show {
            composes,
            operations,
            id,
        } => cmds::cmd_show(&ctx, composes, operations, id),
        PocCmd::Cmp {
            names,
            compact,
            new,
            fork,
        } => cmds::cmd_cmp(&ctx, names, compact, new, fork),
        PocCmd::Status => cmds::cmd_status(&ctx),
        PocCmd::Diff { a, b, stat } => cmds::cmd_diff(&ctx, a, b, stat),
        PocCmd::Config { unset, key, value } => cmds::cmd_config(&ctx, unset, key, value),
    }
}
