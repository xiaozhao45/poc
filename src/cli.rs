//! CLI 分派：全局旗标按位置手工预剥离（命令词之前 = 全局，§5.0 消歧规则），
//! 命令词之后交给各命令解析。四个二进制共用本模块。

use std::process::ExitCode;

use anstream::{eprintln, println};
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
                return Err(PocError::Usage(
                    rust_i18n::t!("error.unknown_global", flag = tok).to_string(),
                ));
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
    about = "P.O.C. — Project·Operation·Compose (safe local operations)"
)]
struct PocCli {
    #[command(subcommand)]
    cmd: PocCmd,
}

#[derive(clap::Subcommand)]
enum PocCmd {
    /// Create or initialize a project
    Proj {
        /// Create a same-named directory here and initialize it
        #[arg(short = 'N', long = "new")]
        new_name: Option<String>,
        /// Initialize at the given path
        path: Option<String>,
    },
    /// Record a new operation (-M/-i) or manipulate existing ones (ids with -f pop / -c compact)
    Opt {
        /// Operation ids (omit to record a new operation)
        ids: Vec<String>,
        /// Operation message (opens an editor when omitted)
        #[arg(short = 'M', long = "message")]
        message: Option<String>,
        /// Include only files matching this glob
        #[arg(short = 'i', long = "include")]
        include: Option<String>,
        /// Pop to the free pool
        #[arg(short = 'f', long = "free")]
        free: bool,
        /// Compact into one operation (head unchanged; originals pooled)
        #[arg(short = 'c', long = "compact")]
        compact: bool,
        /// Rewrite the top operation
        #[arg(long = "amend")]
        amend: bool,
    },
    /// Overview or single operation detail
    Show {
        /// List all composes
        #[arg(short = 'c', long = "composes")]
        composes: bool,
        /// List all operations
        #[arg(short = 'o', long = "operations")]
        operations: bool,
        /// Single operation id
        id: Option<String>,
    },
    /// Compose switching / creation / merge
    Cmp {
        /// Compose names (exactly one = switch; several with -c = merge)
        names: Vec<String>,
        /// Merge composes into a new one (copy)
        #[arg(short = 'c', long = "compact")]
        compact: bool,
        /// Create a new compose
        #[arg(short = 'N', long = "new")]
        new: Option<String>,
        /// Fork: with no id, copies the whole stack from the top; `--fork <op-id>` copies the prefix up to that operation
        #[arg(long = "fork", num_args(0..=1), default_missing_value = "")]
        fork: Option<String>,
    },
    /// Per-compose operation history (append-only; deleting a stack keeps its history)
    Log {
        /// Compose name (default: current stack; deleted stacks remain queryable)
        name: Option<String>,
        /// Show all composes, grouped
        #[arg(short = 'a', long = "all")]
        all: bool,
    },
    /// Free pool / unrecorded changes / pending step
    Status,
    /// Working tree / snapshot differences
    Diff {
        /// One operation id = that operation's diff; two ids = snapshot diff
        a: Option<String>,
        /// Second operation id
        b: Option<String>,
        /// Print per-file stats only
        #[arg(long = "stat")]
        stat: bool,
    },
    /// Repository-level configuration (user identity)
    Config {
        /// Remove a configuration key
        #[arg(long)]
        unset: bool,
        key: Option<String>,
        value: Option<String>,
    },
}

fn argv() -> Vec<String> {
    std::env::args().skip(1).collect()
}

/// help 展示用命令：程序名取 argv[0] 基名（fpoc/dpoc 入口下不再显示 poc），
/// 并追加全局旗标说明（clap 不知道手工预剥离的全局旗标）。
fn build_cmd() -> clap::Command {
    let bin = std::env::args()
        .next()
        .map(|a| {
            std::path::Path::new(&a)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "poc".into())
        })
        .unwrap_or_else(|| "poc".into());
    let mut cmd = PocCli::command();
    cmd.set_bin_name(bin);

    cmd.after_help(rust_i18n::t!("help.after").to_string())
}

fn wrap(r: Res<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e @ PocError::Usage(_)) => {
            eprintln!(
                "{}: {e}",
                crate::theme::paint(crate::theme::Token::Usage, crate::i18n::usage_prefix())
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!(
                "{}: {e}",
                crate::theme::paint(crate::theme::Token::Error, crate::i18n::error_prefix())
            );
            ExitCode::from(1)
        }
    }
}

pub fn poc_main() -> ExitCode {
    crate::i18n::init();
    crate::ui::restore_sigpipe();
    wrap(poc_run(&argv()))
}

pub fn dpoc_main() -> ExitCode {
    crate::i18n::init();
    crate::ui::restore_sigpipe();
    wrap(crate::dcmds::run(&argv()))
}

pub fn cpoc_main() -> ExitCode {
    crate::ui::restore_sigpipe();
    println!("{}", rust_i18n::t!("cpoc.deferred"));
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
    crate::i18n::init();
    crate::ui::restore_sigpipe();
    let args = argv();
    // 顶层 help/version 直接交给 poc_run 的拦截（fpoc 下程序名随 argv[0]）
    if let Some(first) = args.first().map(|s| s.as_str())
        && (matches!(first, "-h" | "--help" | "-V" | "--version")
            || (first == "help" && args.len() == 1))
    {
        return wrap(poc_run(&args));
    }
    let (_, n) = match split_globals(&args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!(
                "{}: {e}",
                crate::theme::paint(crate::theme::Token::Usage, crate::i18n::usage_prefix())
            );
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
    // 顶层 help / version 拦截必须在全局旗标剥离之前（-h 不是全局旗标，
    // 否则 split_globals 会先报"未知全局旗标"）；子命令级 help 仍交给 clap。
    match args.first().map(|s| s.as_str()) {
        Some("-h") | Some("--help") => {
            let _ = build_cmd().print_help();
            return Ok(());
        }
        Some("-V") | Some("--version") => {
            let v = build_cmd().render_version();
            println!("{v}");
            return Ok(());
        }
        Some("help") if args.len() == 1 => {
            let _ = build_cmd().print_help();
            return Ok(());
        }
        _ => {}
    }

    let (g, n) = split_globals(args)?;
    let rest = &args[n..];

    if rest.is_empty() {
        // 裸全局旗标形态（<any command> 可以为空）
        if g.gc {
            let ctx = Ctx::open(g)?;
            let (b, t, o) = ctx.store.gc_run()?;
            println!(
                "{}",
                rust_i18n::t!("gc.done", blobs = b, trees = t, ops = o)
            );
            return Ok(());
        }
        if g.commit {
            let ctx = Ctx::open(g)?;
            return cmds::step_commit(&ctx);
        }
        if g.cancel {
            let ctx = Ctx::open(g)?;
            return cmds::step_cancel(&ctx);
        }
        if g.step {
            let ctx = Ctx::open(g)?;
            match crate::step::load(&ctx.store)? {
                Some(s) => anstream::print!("{}", crate::render::step_section(&s, &ctx.root)),
                None => println!("{}", rust_i18n::t!("status.pending_none")),
            }
            return Ok(());
        }
        let _ = build_cmd().print_help();
        return Ok(());
    }

    // proj 不要求已在项目内，先于 Ctx 分派
    if rest[0] == "proj" {
        let cli =
            PocCli::try_parse_from(std::iter::once("poc".to_string()).chain(rest.iter().cloned()))
                .map_err(|e| PocError::Usage(e.to_string()))?;
        let PocCmd::Proj { new_name, path } = cli.cmd else {
            unreachable!()
        };
        return cmds::cmd_proj(new_name, path);
    }

    let ctx = Ctx::open(g)?;
    if g.gc {
        let (b, t, o) = ctx.store.gc_run()?;
        println!(
            "{}",
            rust_i18n::t!("gc.done", blobs = b, trees = t, ops = o)
        );
    }
    // 带命令的 --commit/--cancel：先处理步骤，再执行命令（"提交步骤结果，进入下一步"）
    if g.commit {
        cmds::step_commit(&ctx)?;
    }
    if g.cancel {
        cmds::step_cancel(&ctx)?;
    }

    let cli =
        PocCli::try_parse_from(std::iter::once("poc".to_string()).chain(rest.iter().cloned()))
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
        PocCmd::Log { name, all } => cmds::cmd_log(&ctx, name, all),
        PocCmd::Diff { a, b, stat } => cmds::cmd_diff(&ctx, a, b, stat),
        PocCmd::Config { unset, key, value } => cmds::cmd_config(&ctx, unset, key, value),
    }
}
