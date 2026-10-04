//! §7.3 性质测试（代数付账处，proptest 随机锤打）。
//! P1 重排不变式 / P2 逆复合 / P3 压缩不变式 / P4 记录幂等 /
//! P6 非破坏不变式 / P7 日志完整。
//! P5（同步收敛）随链接推迟至 cpoc，此处不设（DESIGN §7.3）。
//!
//! 每个 op 独占一个文件（f{i}.txt）⇒ 任意相邻置换无内容冲突（§2.5 判据），
//! 使 P1/P3 可以专注于"合法重排"这一前提本身。

use std::collections::BTreeMap;

use proptest::prelude::*;

use crate::cli::Globals;
use crate::cmds::{self, Ctx, OptArgs};
use crate::hash::Hash;
use crate::log::Event;
use crate::{config, db, log, tree, object, TAG_OP};

fn fixture() -> (tempfile::TempDir, Ctx) {
    // 实验目录约束：一切运行时产物只落在 /tmp/poc 之下
    std::fs::create_dir_all("/tmp/poc").unwrap();
    let dir = tempfile::tempdir_in("/tmp/poc").unwrap();
    let store = cmds::init_at(dir.path()).unwrap();
    store.meta_set(config::USER_NAME, "T").unwrap();
    let ctx = Ctx {
        globals: Globals::default(),
        store,
        root: dir.path().to_path_buf(),
    };
    (dir, ctx)
}

fn args(ids: &[String], free: bool, compact: bool, message: Option<String>) -> OptArgs {
    OptArgs {
        ids: ids.to_vec(),
        message,
        include: None,
        free,
        compact,
        amend: false,
    }
}

/// 记录第 i 个 op：独占文件 f{i}.txt。
fn record(ctx: &Ctx, i: usize, content: &str) {
    std::fs::write(ctx.root.join(format!("f{i}.txt")), content).unwrap();
    cmds::cmd_opt_create(ctx, args(&[], false, false, Some(format!("op{i}")))).unwrap();
}

fn cur(ctx: &Ctx) -> db::ComposeState {
    let name = ctx.store.current_name().unwrap().unwrap();
    ctx.store.compose_get(&name).unwrap().unwrap()
}

fn cur_name(ctx: &Ctx) -> String {
    ctx.store.current_name().unwrap().unwrap()
}

fn pool(ctx: &Ctx) -> Vec<Hash> {
    ctx.store
        .pool_list()
        .unwrap()
        .into_iter()
        .map(|(h, _)| h)
        .collect()
}

fn head_tree_id(ctx: &Ctx) -> Hash {
    cur(ctx).head
}

fn worktree_matches_head(ctx: &Ctx) -> bool {
    let st = cur(ctx);
    let t = ctx.store.get_tree(&st.head).unwrap().unwrap();
    let work = tree::scan(&ctx.root).unwrap();
    tree::disk_matches_tree(&work, &t)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// P1 重排不变式：合法提升后 head 快照不变、栈深不变、当前栈不切换，
    /// 且被共轭替换的原对象仍在（栈 ∪ 池）——P1 与 P6 的合并锤打。
    #[test]
    fn p1_lift_preserves_head(
        contents in proptest::collection::vec("[a-z]{1,6}", 2..=6),
        mask in proptest::collection::vec(any::<bool>(), 2..=6),
    ) {
        let (_d, ctx) = fixture();
        let k = contents.len();
        for (i, c) in contents.iter().enumerate() {
            record(&ctx, i, c);
        }
        let head0 = head_tree_id(&ctx);
        let originals = cur(&ctx).ops.clone();
        let name = cur_name(&ctx);
        prop_assert_eq!(originals.len(), k);
        for i in 0..k {
            if !mask[i % mask.len()] {
                continue;
            }
            let orig = originals[i];
            cmds::opt_lift(&ctx, args(&[orig.hex()], false, false, None), None).unwrap();
            prop_assert_eq!(head_tree_id(&ctx), head0, "提升后 head 必须不变");
            prop_assert_eq!(cur(&ctx).ops.len(), k, "提升不改变栈深");
            prop_assert_eq!(cur_name(&ctx), name.clone(), "提升不切换当前栈");
            let stack = cur(&ctx).ops.clone();
            let pl = pool(&ctx);
            for o in &originals {
                prop_assert!(stack.contains(o) || pl.contains(o), "原 opt 不得消失");
            }
        }
    }

    /// P2 逆复合：opt 后接其逆（前后互换）= 该 op 的前态；全栈逐一取逆 ⇒ base 快照。
    #[test]
    fn p2_inverse_composition(contents in proptest::collection::vec("[a-z]{1,6}", 1..=5)) {
        let (_d, ctx) = fixture();
        let k = contents.len();
        for (i, c) in contents.iter().enumerate() {
            record(&ctx, i, c);
        }
        let mut st = cur(&ctx);
        let base = st.base;
        let name = cur_name(&ctx);
        // 自栈顶向下逐一构造逆 op 并接上（对象层代数，§2.6：逆 = 前后互换）
        for i in (0..k).rev() {
            let top_id = st.ops[i];
            let top = ctx.store.get_op(&top_id).unwrap().unwrap();
            let inv = object::Op {
                pre: top.post,
                post: top.pre,
                author: top.author.clone(),
                msg: format!("inverse of {}", top.msg),
                time_ms: top.time_ms,
            };
            let inv_id = Hash::compute(TAG_OP, &inv.encode());
            let post_tree = ctx.store.get_tree(&inv.post).unwrap().unwrap();
            st.ops.push(inv_id);
            st.origs.push(inv_id);
            st.head = inv.post;
            ctx.store
                .commit_op(&name, &st, &inv, &BTreeMap::new(), &post_tree, None)
                .unwrap();
            prop_assert_eq!(st.head, top.pre, "单步逆复合回到该 op 的前态");
        }
        prop_assert_eq!(st.head, base, "全栈逐一取逆回到 base 快照");
    }

    /// P3 压缩不变式：整栈连续段压缩后 head 快照不变、栈深为 1、原 opts 全部入池。
    #[test]
    fn p3_compact_preserves_head(contents in proptest::collection::vec("[a-z]{1,6}", 2..=6)) {
        let (_d, ctx) = fixture();
        let k = contents.len();
        for (i, c) in contents.iter().enumerate() {
            record(&ctx, i, c);
        }
        let head0 = head_tree_id(&ctx);
        let originals = cur(&ctx).ops.clone();
        let ids: Vec<String> = originals.iter().map(|h| h.hex()).collect();
        cmds::opt_compact(&ctx, args(&ids, false, true, Some("squash".into())), None).unwrap();
        prop_assert_eq!(head_tree_id(&ctx), head0, "压缩后 head 必须不变");
        prop_assert_eq!(cur(&ctx).ops.len(), 1, "整栈压缩后栈深为 1");
        let pl = pool(&ctx);
        for o in &originals {
            prop_assert!(pl.contains(o), "被压缩原 opt 必须入池");
        }
        prop_assert!(worktree_matches_head(&ctx), "压缩不触碰工作区");
    }

    /// P4 记录幂等：opt -M 后工作区快照 == head；无差异时重复记录被拒绝。
    #[test]
    fn p4_record_idempotent(contents in proptest::collection::vec("[a-z]{1,6}\\n", 1..=5)) {
        let (_d, ctx) = fixture();
        for (i, c) in contents.iter().enumerate() {
            std::fs::write(ctx.root.join(format!("f{i}.txt")), c).unwrap();
        }
        cmds::cmd_opt_create(&ctx, args(&[], false, false, Some("all".into()))).unwrap();
        prop_assert!(worktree_matches_head(&ctx), "记录后工作区应与 head 一致");
        let again = cmds::cmd_opt_create(&ctx, args(&[], false, false, Some("again".into())));
        prop_assert!(again.is_err(), "无差异时重复记录必须被拒绝");
        prop_assert!(worktree_matches_head(&ctx));
    }

    /// P6 非破坏不变式：amend 不改 head 树且原 opt 入池；分叉原栈不动；
    /// 合并为副本——源栈不变、当前栈不切换、结果同时含双方变更。
    #[test]
    fn p6_non_destructive(contents in proptest::collection::vec("[a-z]{1,6}", 2..=4)) {
        let (_d, ctx) = fixture();
        for (i, c) in contents.iter().enumerate() {
            record(&ctx, i, c);
        }
        // amend：head 树不变（工作区干净），被替换原 opt 入池
        let head_before = head_tree_id(&ctx);
        let old_top = *cur(&ctx).ops.last().unwrap();
        cmds::cmd_opt_amend(&ctx, args(&[], false, false, Some("amended".into()))).unwrap();
        prop_assert_eq!(head_tree_id(&ctx), head_before, "干净 amend 不改 head 树");
        prop_assert!(pool(&ctx).contains(&old_top), "amend 原 opt 入池");
        // fork：原栈分毫不动，当前切到新栈
        let main_ops = cur(&ctx).ops.clone();
        let main_head = head_tree_id(&ctx);
        cmds::cmd_cmp_new(&ctx, "fork1".into(), Some(String::new())).unwrap();
        prop_assert_eq!(cur_name(&ctx), "fork1");
        prop_assert_eq!(head_tree_id(&ctx), main_head);
        let _ = main_ops;
        // fork1 上加一个 op
        std::fs::write(ctx.root.join("g1.txt"), "g").unwrap();
        cmds::cmd_opt_create(&ctx, args(&[], false, false, Some("fork op".into()))).unwrap();
        // 回 main，加另一个 op
        cmds::cmd_cmp_switch(&ctx, "main".into()).unwrap();
        std::fs::write(ctx.root.join("g2.txt"), "m").unwrap();
        cmds::cmd_opt_create(&ctx, args(&[], false, false, Some("main op".into()))).unwrap();
        let main_st = cur(&ctx);
        let fork1_ops = ctx.store.compose_get("fork1").unwrap().unwrap().ops;
        let name0 = cur_name(&ctx);
        // 合并：副本语义
        cmds::cmd_cmp_merge(&ctx, &["main".to_string(), "fork1".to_string()], Some("m"), None)
            .unwrap();
        prop_assert_eq!(cur_name(&ctx), name0, "合并不切换当前栈");
        let after_main = cur(&ctx);
        prop_assert_eq!(after_main.ops, main_st.ops.clone(), "源栈 main 序列不变");
        prop_assert_eq!(after_main.head, main_st.head, "源栈 main head 不变");
        prop_assert_eq!(
            ctx.store.compose_get("fork1").unwrap().unwrap().ops,
            fork1_ops,
            "源栈 fork1 序列不变"
        );
        let merged = ctx.store.compose_get("m").unwrap().unwrap();
        prop_assert_eq!(merged.ops.len(), main_st.ops.len() + 1, "结果 = main 独有 + 嫁接段");
        let tm = ctx.store.get_tree(&merged.head).unwrap().unwrap().to_map();
        prop_assert!(tm.contains_key("g1.txt"), "结果含 fork 侧变更");
        prop_assert!(tm.contains_key("g2.txt"), "结果含 main 侧变更");
    }

    /// P7 日志完整：每个变更命令恰增对应事件；删栈/purge 后历史行全部保留。
    #[test]
    fn p7_log_complete_and_retained(contents in proptest::collection::vec("[a-z]{1,6}", 2..=4)) {
        let (_d, ctx) = fixture();
        let loglen = || ctx.store.log_list().unwrap().len() as u64;
        let mut expect: u64 = 1; // init 事件
        prop_assert_eq!(loglen(), expect);
        for (i, c) in contents.iter().enumerate() {
            record(&ctx, i, c);
            expect += 1;
            prop_assert_eq!(loglen(), expect, "每次入栈恰增一行");
        }
        cmds::cmd_opt_amend(&ctx, args(&[], false, false, Some("a".into()))).unwrap();
        expect += 1;
        prop_assert_eq!(loglen(), expect);
        let top = *cur(&ctx).ops.last().unwrap();
        cmds::opt_pop(&ctx, args(&[top.hex()], true, false, None), None).unwrap();
        expect += 1;
        prop_assert_eq!(loglen(), expect);
        cmds::cmd_cmp_new(&ctx, "tmp".into(), None).unwrap();
        expect += 1;
        prop_assert_eq!(loglen(), expect);
        // 删栈（dpoc 的 store 层动作）：日志 +1，既有行全部保留
        let before: Vec<Vec<u8>> = ctx
            .store
            .log_list()
            .unwrap()
            .into_iter()
            .map(|(s, e)| {
                let mut v = s.to_le_bytes().to_vec();
                v.extend_from_slice(&e.encode());
                v
            })
            .collect();
        let st = ctx.store.compose_get("tmp").unwrap().unwrap();
        let ev = Event::new(log::EVT_REMOVE, "tmp", None, "", "移除", "", 0);
        ctx.store
            .compose_dispose("tmp", &st.ops, false, "", "compose-remove", &ev)
            .unwrap();
        expect += 1;
        prop_assert_eq!(loglen(), expect);
        let after = ctx.store.log_list().unwrap();
        prop_assert_eq!(after.len() as u64, expect);
        for (i, b) in before.iter().enumerate() {
            let (s, e) = &after[i];
            let mut v = s.to_le_bytes().to_vec();
            v.extend_from_slice(&e.encode());
            prop_assert_eq!(&v, b, "删栈后历史行全部保留");
        }
        // purge：真删除 op，日志行仍保留（痕迹永久）
        let victim = pool(&ctx)[0];
        let evd = Event::new(log::EVT_DESTROY, log::POOL_SCOPE, Some(victim), "", "d", "", 0);
        ctx.store
            .destroy_from_pool(&victim, "自由池", "opt-destroy", &evd)
            .unwrap();
        let evp = Event::new(log::EVT_PURGE, log::POOL_SCOPE, Some(victim), "", "p", "", 0);
        let before2: Vec<Vec<u8>> = ctx
            .store
            .log_list()
            .unwrap()
            .into_iter()
            .map(|(s, e)| {
                let mut v = s.to_le_bytes().to_vec();
                v.extend_from_slice(&e.encode());
                v
            })
            .collect();
        ctx.store.purge_op_row(&victim, &evp).unwrap();
        let after2 = ctx.store.log_list().unwrap();
        prop_assert_eq!(after2.len() as u64, before2.len() as u64 + 1);
        for (i, b) in before2.iter().enumerate() {
            let (s, e) = &after2[i];
            let mut v = s.to_le_bytes().to_vec();
            v.extend_from_slice(&e.encode());
            prop_assert_eq!(&v, b, "purge 后历史行保留");
        }
        prop_assert!(ctx.store.get_op(&victim).unwrap().is_none(), "purge 后对象确已删除");
    }
}
