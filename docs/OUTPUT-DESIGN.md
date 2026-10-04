# P.O.C. 输出改造设计（UI Overhaul）

- 状态：设计稿 v1，待评审
- 日期：2026-10-02
- 输入：终端输出现状调查（/tmp/poc/output-survey.md、/tmp/poc/output-samples.txt）
  + 评审意见（颜色、i18n、禁表格、禁终端宽度依赖、去黑话、去缩写、C/E/F/G/H/I/J/K 全修）

## 0. 评审意见 → 设计约束

| # | 意见 | 约束 |
|---|---|---|
| 1 | 颜色该整治，要跨平台 | 用 anstyle + anstream（已是依赖）建主题层 |
| 2 | 默认英文，做跨平台 i18n，消息提取出来方便贡献 | rust-i18n 目录化，en 为事实源 |
| 3 | 不用表格；不做任何依赖终端尺寸的输出 | 弃 comfy-table / indicatif；对齐只允许基于内容 |
| 4 | 信息别挤一行，用换行和缩进 | 逐命令输出规范（§4）全部改为多行缩进式 |
| 5 | "整体/内部/hunks" 是代数黑话，前端不出现 | 前端只说 added/deleted/updated 文件数 |
| 6 | "opt" 缩写不正式 | 面向用户的文案一律写全称 operation |
| 7 | --stat 竖线要在同一直线上 | 路径列按最长路径补齐（内容对齐，不涉终端宽度） |
| C/E/F/G/H/I/J/K | 见调查稿清单 | §5 逐项落地 |

术语政策：**Project / Operation / Compose 三个产品名词在一切语言里保持英文原词**；
其余文案全部进消息目录。`opt`/`cmp` 等作为 CLI 命令词不变，但出现在文案里时
写全称（如 "operation b05a01c0"，命令名只在反引号代码体里出现）。

## 1. 依赖选型盘点（用 crate 借力，但按约束筛）

| 用途 | 选定 | 理由 |
|---|---|---|
| 颜色 | **anstyle + anstream**（已在 Cargo.toml，当前零使用） | rust-cli 官方生态件，cargo/rustup 同款；纯 Rust、三平台；anstream 自动处理 TTY 检测、管道剥离、NO_COLOR / CLICOLOR_FORCE，正好满足"跨平台颜色"且不用手写降级逻辑 |
| i18n | **rust-i18n** | 编译期把 locales/*.yml 嵌入二进制（保住"单二进制"目标），`t!("id", args)` 取词；YAML 目录即"提取出来的文档"，贡献者加一个 `locales/xx.yml` 即可；单一纯 Rust 依赖 |
| 表格 | 弃（comfy-table 等） | 评审明确不要表格 |
| 进度条 | 弃（indicatif） | 全部操作是亚秒级本地事务，无进度可画；且其布局依赖终端宽度，违反约束 |
| 备选说明 | owo-colors（与 anstyle 功能重叠，不引入）、gettext-rs（C FFI，违反 DESIGN §7.2 纯 Rust 依赖原则）、fluent-templates（功能强但引入 4-5 个 crate，运行时加载复杂，对本项目 ~260 条消息过重） | |

**新增依赖合计：1 个（rust-i18n）。**

## 2. i18n 架构

### 2.1 目录与取词

```
locales/
  en.yml        # 事实源（canonical），键 = 消息 id，贡献者从这里抄结构
  zh-CN.yml     # 第一份翻译（现役文案的中文化）
```

- 取词统一走 `t!("record.created", id = …, msg = …)`；缺键自动回退 en。
- 键按域组织：`record.*` / `show.*` / `status.*` / `log.*` / `diff.*` / `cmp.*` /
  `stack.*`（lift/pop/compact/amend 收尾）/ `step.*` / `error.*` / `usage.*` /
  `help.*` / `gc.*` / `config.*`。
- 消息面盘点（改造工作量）：cmds 173、dcmds 48、cli 14、err 15、render 3、
  step 2、ui 4 ≈ **260 处**。

### 2.2 语言选择链

`POC_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `en`。
值形如 `en` / `zh-CN`；不在 `rust_i18n::available_locales!()` 里一律回退 en。
（不引 sys-locale，直接读环境变量，保依赖最小。）

### 2.3 错误的本地化（实现定稿）

- `err.rs` 的 `PocError` 保持**类型化枚举**（携带 id、路径等数据）；thiserror 的
  `#[error("…")]` 改写为**英文规范文案**，仅作 Debug/兜底显示；
- 可本地化的错误**在构造点**经 `t!` 生成（如 `PocError::Msg(t!("error.op_not_found", id = s))`），
  与普通消息同一通道迁移；
- `cli.rs::wrap()` 只负责前缀与退出码：`error:`（exit 1）/ `usage:`（exit 2），
  前缀词本身经目录本地化（en: error / usage，zh: 错误 / 用法错误）；
- 由此消灭"错误：未找到：opt 前缀 zz99"式前缀堆叠：输出为
  `error: operation not found: zz99`。

### 2.4 已知限制（明示）

- clap 自身的动态错误（unrecognized subcommand 等）不经过目录，恒为英文；
  en 为默认语言时这是自洽的，zh 下混英文属已知限制，留待后续（可选项：
  预校验子命令词，或手写错误渲染绕开 clap）。
- clap 帮助文本（子命令 about 等 doc comment）随 2 改为英文；zh 不翻译帮助
  正文（限制同上）。
- 时间/数字一律 ISO `YYYY-MM-DD HH:MM` + ASCII 数字，不做本地化格式。

### 2.5 持久化文案的本地化边界

- 事件 kind 是结构化字段（u8），展示时经目录本地化；
- 日志事件的 msg/detail 是**持久化数据**：落库时写英文规范文案（代码常量），
  展示层不翻译——历史行不随语言切换漂移；
- op 的 author/msg 是用户内容，永不翻译。

## 3. 主题层（颜色）

新增 `src/theme.rs`，全程序唯一的样式出口：

| token | 样式 | 用途 |
|---|---|---|
| `error` | 红 + 加粗 | `error:` 前缀 |
| `usage` | 黄 + 加粗 | `usage:` 前缀（用法错误，exit 2） |
| `ok` | 绿 | 动词结果行（created / switched / …） |
| `id` | 青 | 8 位短 id、64 位全 id |
| `added` / `removed` / `updated` | 绿 / 红 / 黄 | `+ path`、`- path`、`M path` 行与计数词 |
| `header` | 加粗 | 小节标题行（compose …、free pool …、history …） |
| `dim` | 暗淡 | 详情尾巴（` — …` 之后的部分）、作者行、来源标注 |
| `quote` | 默认 | `"msg"`（不着色，消息本身是内容） |

管线：

- 所有 stdout 输出经 `anstream::AutoStream`（`anstream::println!` / `print!`），
  非TTY / 管道 / NO_COLOR 自动剥离转义，stderr 同理（anstream::eprintln!）。
- 字符串组装用 `style.render()` / `render_reset()` 包裹片段；`theme.rs` 提供
  `paint(token, text) -> String` 小助手，避免散落的转义码。
- **验证点（实现期）**：minus pager 路径的 ANSI 透传。若 minus 5.8 不能干净
  透传，就在 `ui::page` 进 pager 前用 `anstream::adapter::strip_str` 剥离
  （展示型大块输出本就以"可读"优先，降级无损失）。
- 颜色不承载信息：任何去掉颜色后必须语义不变（无障碍/重定向友好）。

## 4. 输出规范（默认英文；对齐只基于内容；层级只用换行+缩进）

### 4.0 硬规则（适用于一切命令）

1. 不出现表格线、不按终端宽度折行/截断；对齐列宽只由内容（最长键/路径）决定。
2. 前端词汇：`operation`（不写 opt）、`compose`、`free pool`；
   不出现 整体/内部、hunk、conjugate 等内部术语；
   计数只用 `N added, N deleted, N updated`（文件级）。
3. 减号一律 ASCII `-`；**箭头一律 ASCII `->`**（head 变化、swap 文件映射、日志
   detail）；省略号 `…` 只用于截断 64 位 id 的展示场景。
4. 层级定式：**小节标题一律顶格**（compose / stack / free pool / unrecorded
   changes / pending step / history …），正文缩进 2 空格，条目的次级信息
   （作者行等）缩进 4 空格，**禁止更深嵌套——小节绝不嵌在小节里**（评审定稿：
   `stack` 与 `compose` 同级、`unrecorded changes` 与 `compose` 同级）。
5. 每条命令输出 = **结果行（动宾结构，含 id）+ 缩进详情块**；错误单独一行
   `error: …` / `usage: …`，不带第二前缀。

### 4.1 record（`poc opt -M`）

```
$ poc opt -M "init: README + app"
created operation b05a01c0 "init: README + app"
  2 added

$ poc --verbose opt -M "app v2" -i "app.rs"
created operation 2f654c08 "app v2"
  1 updated
  M app.rs

$ poc --dry-run opt -M "would record"
dry run: would create operation a98b8c82 "would record" on compose "main" (nothing written)
  1 added

$ poc --quiet opt -M "…"
b05a01c0
```

- 计数行缺的类别直接省略；三动词着色（added 绿 / deleted 红 / updated 黄）。
- 文件清单从默认输出移到 `--verbose`（status 里本来就常驻清单，避免双份）。

### 4.2 show（栈图，多行缩进式）

```
$ poc show
compose main (current)
  base:  b18b177d
  head:  6115a54d

stack (3 operations, top first):
  b05a01c0  "app v2"
    demo <demo@example.com> · 2026-10-02 11:04
  16e1315f  "squash two"
    demo <demo@example.com> · 2026-10-02 11:05
  f4eae991  "notes added, README removed"
    demo <demo@example.com> · 2026-10-02 11:03
```

- 空栈：`stack is empty (head == base)`；id 着 `id` 色，作者/时间行着 `dim`。
- `stack` 与 `compose` 同级顶格（评审定稿）；删除现版的 `top --` / 孤立 `|` /
  9 空格竖线画法。

### 4.3 show -c（每个 compose 一个块）

```
$ poc show -c
compose main (current)
  base:  b18b177d
  head:  6115a54d
  3 operations

compose exp
  base:  c175a0dc
  head:  c175a0dc
  empty stack
```

### 4.4 show <id>（详情；元数据缩进，diff 保持原样便于复制）

```
$ poc show 2f654c08
operation 2f654c08b2381031672d40be468d36ef1207fc3540306702b3f2e543e7b86181
  message:  app v2
  author:   demo <demo@example.com>
  time:     2026-10-02 11:04
  pre:      6115a54ddc453f1583f1da05b90a29292c0f60379e57e3b597281f3a043196db
  post:     e3fa687065790ff4b55fe060e36855c1afb6edc9b6bdafd65b841225a1b5f5cd

--- a/app.rs
+++ b/app.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!("v1");
+    println!("v2");
```

- 键列按最长键补齐（内容对齐）；diff 部分不缩进、不着色（保 copy-patch 可用）。

### 4.5 status

```
$ poc status
free pool (7, oldest first):
  2026-10-02 11:04  2f654c08  "app v2"
  2026-10-02 11:04  b05a01c0  "init: README + app"
  …
  （空时显示 `free pool (0): empty`）

compose main (current)
  base:  b18b177d
  head:  6115a54d

unrecorded changes: 1 added, 1 deleted, 1 updated
  + notes.txt
  - README.md
  M app.rs

pending step: none
```

- 四节同构：`标题行顶格 + 两格缩进体`（F；评审定稿：`unrecorded changes` 与
  `compose` 同级顶格，不嵌在 compose 节内）；
- 干净态：unrecorded changes 节显示 `unrecorded changes: none (working tree
  matches head)`；
- 池条目时间前置（定宽），不再需要任何列对齐，信息量 +1（F）。

### 4.6 log

```
$ poc log
history of "main" (oldest first):
  #1    2026-10-02 11:04  init     proj initialized — base = empty snapshot
  #2    2026-10-02 11:04  push     b05a01c0  demo  "init: README + app"
  #7    2026-10-02 11:04  push     b05a01c0  demo  "lifted b05a01c0 to top" — 2 copies re-created, head -> f1f2c859
  #8    2026-10-02 11:04  pop      b05a01c0  — head -> 5cf54f13
```

- 行结构：`#seq(宽4) time kind(左对齐宽7) [id] [author] ["msg"][ — detail(dim)]`；
  **无 op 的事件不再放 `········` 占位**，id 段直接缺席（E）；
- kind 词表（英文小写，zh 目录给对应词）：init / new-stack / push / amend /
  compact / pop / merge / remove / destroy / restore / purge；
- 重复词修复在**事件文案层**完成：cmds 里构造 Event 时 msg 不再复述动词
  （"提升 b05a01c0" → "lifted b05a01c0 to top"）（E）；
- `log -a`：`history of "main" (oldest first):` 按栈分块，块间空一行。

### 4.7 diff

- 全文：unified 格式原样（`--- a/p` / `+++ b/p` / `@@`），新增/删除/修改三段
  顺序不变，颜色加在 `+`/`-` 行上（added/removed token）；
- `--stat`（I：竖线同一列；去冗余 kind 列；二进制不再显示 +0 -0）：

```
$ poc diff --stat
  README.md  | +0 -3
  notes.txt  | +1 -0
  app.rs     | +2 -1
  app.bin    | (binary)
```

### 4.8 cmp 族（结果行 + 缩进详情；H 的"两行式"推广为统一模式）

```
switched to compose "main" (head 6115a54d)
already on compose "main"
created compose "exp" and switched to it (new history, base = head of "main")
created compose "pre" and switched to it (forked from "main" at 2f654c08, 2 operations)
  head f1f2c859 -> e3fa6870 (working tree updated)
merged "main" + "side" into new compose "merged" (6 operations)
  head 90b2c51d
  the source composes are kept; view it with `poc show merged`, switch with `poc cmp merged`
```

### 4.9 栈操控统一收尾（H：lift / pop / compact / amend 共用同一模式）

三行定式：**结果行 → head 行 → pool 行**，用词唯一：

```
lifted b05a01c0 to the top of "main"
head unchanged (f1f2c859)
pooled 2 operations: 16e1315f, b05a01c0

applied b05a01c0 onto "exp"
head c175a0dc -> 20b421a0 (working tree updated)
nothing pooled

popped b05a01c0 off "main"
head f1f2c859 -> 5cf54f13 (working tree updated)
pooled 1 operation: b05a01c0

compacted 2 operations into 16e1315f "squash two" (in place)
head unchanged (f1f2c859)
pooled 2 operations: b05a01c0, 2f654c08

created composite 3e62801e "pool fold" in the free pool
head unchanged (fa890a19)
the stack is untouched

rewrote the top operation of "main": 6545772e "app v3 (amended)"
head 5cf54f13 -> 6545772e (working tree updated)
pooled 1 operation: 55f9b26a
```

- "conjugate（共轭）"不出现；池内/他栈 opt 上栈叫 `applied`，栈内重排叫
  `lifted … to the top`；`--verbose` 时追加一行列出新建的中间 operation id。

### 4.10 步骤协议（K：单渲染器，`poc -s` 与 status 共用）

```
pending step: compact
  operations: c1a0950e 039bac17
  message:    squash
  c.txt -> .poc/swap/step/0000__c.txt
  edit the swap files to remove conflict markers, then run `poc --commit`;
  discard with `poc --cancel`
```

- 建步骤输出（exit 1）：
  `error: conflict cannot be resolved automatically; a pending step was left`
  + 上面同一渲染器的缩进体。裸 `poc -s` 无步骤时：`pending step: none`。
- 提交：业务结果行照常输出，收尾 `step committed; swap files cleaned up`。

### 4.11 错误与帮助（G / C）

```
error: not in a P.O.C. project (no .poc/store found; run `poc proj` to initialize)
error: working tree is not clean; record changes first with `poc opt -M`
error: operation with prefix "zz99" not found
usage: `-f` and `-c` require operation ids
```

帮助（程序名取 argv[0] 基名，fpoc/dpoc 下不再显示 poc；全局旗标补进 after_help）：

```
P.O.C. — Project·Operation·Compose

Usage: poc <COMMAND>
…（子命令表，doc comment 改英文）…

Global options (place them before the command word):
  -o, --gc         collect garbage first
  -s, --step       never open an editor or UI; leave a pending step instead
  -c, --commit     commit the pending step, then run the command
  -q, --cancel     discard the pending step
  -p, --no-pager   disable the embedded pager
      --dry-run    rebuild in memory and report, without writing anything
      --quiet      print only result ids and errors
      --verbose    print file lists and step-by-step details
  -y, --yes        (reserved; no effect in 1.0)

Flags placed after the command word belong to that command (`-c` = compact
after `opt`, merge after `cmp`).
```

### 4.12 gc / config

```
gc: removed 0 blobs, 0 trees, 0 operations

effective configuration:
  user.name     demo             (repository)
  user.email    demo@example.com (repository)
  project.name  outdemo          (set at init)
```

## 5. 评审清单逐项落点

| 项 | 修改 | 落点 |
|---|---|---|
| C | 动态程序名；全局旗标进 after_help | cli.rs（`PocCli::command()` 按 argv[0] 设 name；after_help 经 t!） |
| E | log 行去占位/去重复；事件 msg 不复述动词 | log.rs `Event::line` + cmds/dcmds 事件构造处 |
| F | status 三节同构 + 池条目加时间 | cmds.rs `cmd_status` + render.rs |
| G | 前缀堆叠消灭、ASCII 化、全角清理 | err.rs 渲染层 + 全部文案（随 i18n 顺产） |
| H | 栈操控三行定式 | cmds.rs 四个出口 + render.rs 新增 `stack_outcome` 助手 |
| I | --stat 竖线对齐、去 kind 列、binary 标注 | cmds.rs `diff_text` 的 emit |
| J | --quiet 落地（只出 id 与错误）；--verbose 扩到栈操控细节 | Globals 消费点：cmds.rs 各 println 加 `if !quiet` |
| K | `-s` 视图与 status 步骤节共用渲染器 | render.rs `step_section` + cli.rs 裸 `-s` 分支 |
| B | 默认英文 + 目录化 | rust-i18n 接线，全部 println 过 t! |
| A | 颜色 | theme.rs + anstream 路由 |

连带必改：**docs/acceptance-demo.sh** 的 grep 目标（"自由池（1）"、`top --`、
"压缩"等全部换成新文案）；docs/DESIGN.md §5 的示例会话同步重录。

## 6. 实施切分（工作区提交 → 自托管库逐个记 Operation）

| Op | 内容 | 验证 |
|---|---|---|
| O1 | i18n 骨架：rust-i18n 依赖、locales/en.yml + zh-CN.yml、t! 接线、错误渲染层、语言链 | 全命令冒烟；`POC_LANG=zh-CN` 对照 |
| O2 | theme.rs + anstream 全路由（含 pager 透传验证/降级） | TTY/管道/NO_COLOR 三态肉眼+脚本 |
| O3 | render.rs 重写：栈图/compose 块/详情块/pool 行/log 行/stat 对齐 | collect.sh 重跑比对 |
| O4 | cmds/dcmds 文案统一（4.1/4.8/4.9/4.10/4.12）+ J 语义 + C 帮助 | collect.sh + acceptance-demo.sh 更新后通过 |
| O5 | 收尾：collect.sh 黄金样本重录、DESIGN.md §5 示例同步、/tmp/poc/self 逐 Op 记录验证 | self 库 `poc log` 逐条核对 |

每个 Op 独立可编译、可演示、可回退；工作区 git 提交与 /tmp/poc/self 的
`poc opt -M` 一一对应。
