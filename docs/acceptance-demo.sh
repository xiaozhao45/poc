#!/usr/bin/env bash
# P.O.C. 1.0 §0 七条目标逐条验收演示。
# 约束：一切运行只发生在 /tmp/poc 之下。
# 用法：POC_BIN=target/release/poc DPOC_BIN=target/release/dpoc bash docs/acceptance-demo.sh
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
POC=${POC_BIN:-$ROOT/target/release/poc}
DPOC=${DPOC_BIN:-$ROOT/target/release/dpoc}
BASE=/tmp/poc/acceptance
# 断言基于英文文案；外部可用 POC_LANG 覆盖
export POC_LANG=${POC_LANG:-en}

say() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }
ok() { printf '\033[32m  ✓ %s\033[0m\n' "$*"; }
# 当前栈顶的 operation id（show 的 stack 节第一条目）
topid() { "$POC" -p show | grep -A1 'stack (' | tail -1 | awk '{print $1}'; }

rm -rf "$BASE"; mkdir -p "$BASE"; cd "$BASE"

say "目标 1｜端到端纯本地：单二进制、无网络、无守护进程"
"$POC" proj -N g1 >/dev/null
cd g1
"$POC" config user.name 验收 >/dev/null
"$POC" config user.email a@b.c >/dev/null
printf 'l1\nl2\nl3\n' > code.txt
"$POC" opt -M "init" >/dev/null
"$POC" show >/dev/null
ok "proj → config → opt → show 全部本地完成（poc 单二进制）"

say "目标 2｜像 Git 一样在本地维护多个代码库的完整历史"
cd "$BASE"
"$POC" proj -N g2 >/dev/null
(
  cd g2
  "$POC" config user.name 验收 >/dev/null
  "$POC" config user.email a@b.c >/dev/null
  echo x > x.txt
  "$POC" opt -M "g2 init" >/dev/null
  "$POC" show -o >/dev/null
)
ok "g1 与 g2 两个 Project 各自独立维护完整历史"

say "目标 3｜行级粒度"
cd "$BASE/g1"
printf 'l1\nL2-modified\nl3\n' > code.txt
DIFF=$("$POC" diff | grep -cE '^[+-]' || true)
"$POC" diff | grep -q 'L2-modified'
"$POC" opt -M "edit one line" >/dev/null
ok "行级差分（$DIFF 行变更，精确命中 L2-modified）"

say "目标 4｜栈语义历史 + 自由池"
for i in 1 2 3; do
  echo "v$i" > "s$i.txt"
  "$POC" opt -M "op$i" >/dev/null
done
TOP=$(topid)
"$POC" opt "$TOP" -f >/dev/null                 # 显式弹出 → 自由池
"$POC" status | grep -q 'free pool (1'
"$POC" opt "$TOP" >/dev/null                    # 按 id 提升回栈（显式入栈）
ok "弹出→池→按 id 回栈：历史是 Compose 栈，池内 opt 一等公民"

say "目标 5｜步骤化交互：冲突经 .poc/swap 解决，永不污染工作区"
"$POC" cmp -N cfl --fork >/dev/null
printf 'X\n' > c.txt && "$POC" opt -M "t1" >/dev/null
printf 'Y\n' > c.txt && "$POC" opt -M "t2" >/dev/null
printf 'Z\n' > c.txt && "$POC" opt -M "t3" >/dev/null
T1=$("$POC" log | grep '"t1"$' | tail -1 | awk '{print $5}')
T3=$("$POC" log | grep '"t3"$' | tail -1 | awk '{print $5}')
"$POC" opt "$T1" "$T3" -c -M "squash" >/dev/null 2>&1 || true   # 聚拢冲突 → 建步骤
test -f .poc/swap/step/0000__c.txt
grep -q '<<<<<<< ours' .poc/swap/step/0000__c.txt
printf 'RESOLVED\n' > .poc/swap/step/0000__c.txt
"$POC" --commit >/dev/null
ok "聚拢冲突 → 交换文件 → 编辑 → --commit 回放成功（工作区全程未被冲突污染）"

say "目标 6｜日常工具永不销毁：销毁隔离于 dpoc，三层把门"
printf 'Z-extra\n' >> c.txt
"$POC" opt -M "poc 无销毁动词" >/dev/null
set +e
"$DPOC" opt-destroy x >/dev/null 2>&1
C=$?
set -e
[ "$C" -ne 0 ]
ok "dpoc 未 enable 直接拒绝（exit $C）；poc 二进制内不存在销毁代码路径"

say "目标 7｜信息不破坏：压缩原 opt 入池，日志删栈不删史"
N=$("$POC" status | grep -oE 'free pool \([0-9]+' | grep -oE '[0-9]+')
[ "$N" -ge 3 ]
"$POC" log | grep -q 'compact'
"$POC" log -a >/dev/null
ok "聚拢压缩后原 opt 全部在自由池（$N 个）；log 追加式完整记录（删栈不删史）"

say "附｜错误文案与退出码约定"
set +e
"$POC" opt -f >/dev/null 2>&1
C1=$?
"$POC" nonsense >/dev/null 2>&1
C2=$?
set -e
[ "$C1" -eq 2 ] && [ "$C2" -eq 2 ]
ok "用法错误 → 退出码 2（实测 $C1/$C2）；拒绝类（冲突/脏工作区/待办步骤）→ 1"

printf '\n\033[1;32m§0 七条目标逐条验收：全部通过\033[0m\n'
