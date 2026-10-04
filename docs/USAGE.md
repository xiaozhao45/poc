# P.O.C. 使用手册

**P.O.C.**（Project · Operation · Compose）是一个用户态、纯本地的版本控制工具：单二进制、无网络、无守护进程、无暂存区。它像 Git 一样为一个目录维护完整历史，但历史模型不同——历史是一摞**可以自由重排的 Operation 栈**，而不是一条提交链。

本手册自包含：使用 poc 所需的一切都在这一篇里。

---

## 1. 核心概念

| 概念 | 是什么 |
|---|---|
| **Project** | 一个代码库 = 一个含 `.poc/` 目录的文件夹。身份就是它的路径。 |
| **Operation（op）** | 一次最小变更。自包含"前态/后态"一对快照，不依赖父节点；内容寻址，id 是 64 位十六进制，日常用前 8 位。 |
| **Compose（栈）** | 有序的 Operation 栈：base 快照 + 自底向上的 op 序列，栈全部作用后的快照就是 head。类似分支，但可重排、可压缩、可合并。 |
| **自由池** | 每项目一个，收纳不属于任何栈的 op。池内 op 是一等公民，随时可按 id 回栈。 |
| **步骤协议** | 一切交互（编辑器、冲突处理）都可以被拆成非交互步骤；冲突永远不污染工作区，走交换文件。 |

目录布局（在项目内自动生成）：

```
你的项目/
  .poc/store     # 全部数据（单文件库）
  .poc/swap/     # 步骤交换文件（会话性，可随时清空）
  .pocignore     # 可选：忽略规则（glob 语法；不读 .gitignore）
  （你的工作区文件）
```

追踪规则：`.poc/` 自身永不追踪；隐藏文件不追踪；空目录不追踪；文件字节原样存储，不改换行符；二进制文件不产生行级差异。

---

## 2. 五分钟上手

```console
$ poc proj -N demo            # 新建目录 demo 并初始化（Compose: main）
$ cd demo
$ poc config user.name "你的名字"
$ poc config user.email "you@example.com"
$ echo "# demo" > README.md
$ poc opt -M "init"
  1 added
  created operation 3f9c0a1b "init"
$ poc show
compose main (current)
  base:  b18b177d
  head:  6115a54d

stack (top first, 1 total):
  3f9c0a1b  "init"
    你的名字 <you@example.com> · 2026-10-02 11:04
$ poc log
history of "main" (oldest first):
  #1    2026-10-02 11:04  init     proj initialized — base = empty snapshot
  #2    2026-10-02 11:04  push     3f9c0a1b  你的名字  "init"
```

日常循环就是：改文件 → `poc opt -M "说明"` → 继续干活。

---

## 3. 全局旗标与命令词

```
poc [全局旗标] [命令 [参数…]]
```

**位置规则**：全局旗标必须写在命令词**之前**；命令词之后的同名短旗标属于该命令。例如 `-c` 写在 `poc` 后是"提交步骤"，写在 `opt` 后是"压缩"，写在 `cmp` 后是"合并"。

| 全局旗标 | 作用 |
|---|---|
| `-o`, `--gc` | 执行命令前先做垃圾回收（单独 `poc -o` 就是只 gc） |
| `-s`, `--step` | 非交互模式：不打开任何编辑器/界面，需要交互时留下待办步骤后退出 |
| `-c`, `--commit` | 提交待办步骤的结果（可带命令：先提交步骤再执行命令） |
| `-q`, `--cancel` | 放弃待办步骤 |
| `-p`, `--no-pager` | 本次输出不分页 |
| `--dry-run` | 预演：完整内存重建并报告，但不落盘 |
| `--quiet` | 只输出结果 id 与错误 |
| `--verbose` | 额外输出文件清单、新铸 operation 明细等细节 |
| `-y`, `--yes` | 预留，1.0 无效果 |

环境变量：`POC_LANG`（界面语言，默认 `en`，内置 `zh-CN`；也接受 `LC_ALL`/`LC_MESSAGES`/`LANG`）、`POC_EDITOR` → `VISUAL` → `EDITOR`（编辑器选择链，缺省 vi/notepad）、`POC_CONFIG_DIR`（dpoc 许可文件位置）。终端颜色自动管理：TTY 保留、管道剥离、`NO_COLOR` 生效。

退出码：**0** 成功；**1** 拒绝（冲突、脏工作区、待办步骤、未找到）；**2** 用法错误。

---

## 4. 命令参考

### 4.1 `proj` —— 初始化项目

```
poc proj              # 当前目录原地初始化
poc proj <path>       # 在指定目录初始化（不存在则创建）
poc proj -N <name>    # 在当前目录下新建同名文件夹并初始化
```

初始化会创建一个名为 `main` 的 Compose（base = 空快照）。重复初始化报错。

### 4.2 `config` —— 用户信息

```
poc config                          # 列出生效值及来源
poc config user.name "名字"          # 写入本仓库
poc config user.email "a@b.c"
poc config --unset user.name
```

记录 operation 时作者信息的解析链：本仓库 → 环境变量 `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` → Git 全局配置 → 报错。**name 必须可解析，否则无法记录。** 支持的键只有 `user.name` 和 `user.email`。

### 4.3 `opt` —— 记录与操控 Operation

**记录（入栈原语）：**

```
poc opt -M "消息" [-i "文件 glob"]
```

把工作区相对 head 的变更铸成一个新 op 压入当前栈顶。没有暂存区：记录的就是工作区现状。`-i` 用 glob 限定只收匹配的文件（如 `-i "src/*"`），未选中的文件保持未记录状态。省略 `-M` 会打开编辑器写消息（`-s` 模式下必须显式给 `-M`）。

**操控既有 op**（id 用 ≥4 位十六进制唯一前缀，可命中栈内或池内）：

```
poc opt <ids…>            # 提升
poc opt <ids…> -f         # 弹出到自由池
poc opt <ids…> -c -M "消息" # 压缩为一个
poc opt --amend [-M "消息"] # 改写栈顶
```

- **提升**：按给定顺序逐个处理。op 在当前栈上 → 重排到栈顶（head 不变）；op 在他栈或池内 → 把它的变更施加到当前栈顶（head 延长，工作区物化）。这是从别的栈"捡变更"的原语。
- **弹出 `-f`**：把命中的 op 移出当前栈、放入自由池，head 回退重算、工作区物化（被删掉的文件会真的消失——这就是"回退"）。池内的 op 无需弹出；他栈的 op 不能跨栈弹出。
- **压缩 `-c`**（至少两个不同 op）：
  - 命中的恰为当前栈**连续段** → 精确复合、**原位替换**，head 不变；
  - 不连续 → 先自动**聚拢**到一起再压缩（若两个 op 改了同一文件的同几行，聚拢会冲突 → 步骤协议），head 仍不变；
  - 含池内/他栈 op → 按给定顺序做对象级复合，**结果放入自由池**，当前栈分毫不动（可用 `poc opt <结果id>` 决定是否入栈）。
  三种情况下被替换的原 op 全部转入自由池，随时可重组。
- **改写 `--amend`**：以（栈顶的前态，工作区快照）替换栈顶 op，工作区变更一并并入；原栈顶 op 转入自由池。只改消息时 head 不变。

工作区不干净（与 head 有差异）时，提升/弹出/压缩/切换都会被拒绝；先 `poc opt -M` 记录，或用 `--dry-run` 预演。

### 4.4 `show` —— 看状态

```
poc show            # 当前 compose 头 + 栈（栈顶在前）
poc show -c         # 所有 compose，各一个块
poc show -o         # 全部 operation，按时间倒序
poc show <id>       # 单个 op 详情：元数据 + 行级差异全文
```

### 4.5 `cmp` —— Compose 管理

```
poc cmp <name>                        # 切换（需工作区干净；物化目标 head）
poc cmp -N <name>                     # 新建：base = 当前 head、空栈（"新历史"）
poc cmp -N <name> --fork              # 新建：复刻当前整栈（"平行历史"）
poc cmp -N <name> --fork <op-id>      # 新建：复刻自底到该 op 的前缀（原栈不动）
poc cmp <a> <b> [更多名…] -c [-N 结果名]   # 合并为一个新栈（副本）
```

- 合并构造：公共前缀共享 op 对象，其余各栈独有段逐个嫁接。**被合并的栈原样保留，当前栈也不切换**；结果栈名缺省 `merged`（重名自动 `merged-2`…）。合并冲突 → 步骤协议，整体不落地。
- 三种新建的取舍：不带 `--fork` = 从当前状态出发、历史从零算（类 orphan 分支）；`--fork` = 连历史一起复刻（类 `checkout -b`）；`--fork <op-id>` = 只要某一点之前的历史。

### 4.6 `status` —— 自由池 / 未记录变更 / 待办步骤

```
poc status
```

三节：自由池（按入池时间）、当前 compose 与未记录变更（+新增 / -删除 / M 修改，逐文件）、待办步骤。

### 4.7 `diff` —— 差异

```
poc diff              # 工作区 vs 当前 head（unified 全文）
poc diff --stat       # 逐文件统计（竖线对齐；二进制文件标注）
poc diff <id>         # 该 op 的差异
poc diff <idA> <idB>  # 两个 op 的后态快照之间的差异
```

### 4.8 `log` —— 追加式操作日志

```
poc log               # 当前栈的编年史（旧 → 新）
poc log <name>        # 任意栈（包括已被删除的栈）
poc log -a            # 全部栈分组（池内事件挂在 "(池)" 名下）
```

日志只增不改不删，与每条业务变更同一事务落盘：**删栈不删史**。每行：序号、时间、事件种类（init / new-stack / push / amend / compact / pop / merge / remove / destroy / restore / purge）、op id、消息与详情。

### 4.9 `gc`

```
poc -o
```

清除不被任何 compose、自由池或活动步骤引用的对象。池内对象永不清；非破坏原则下 gc 实际只是兜底。

---

## 5. 步骤协议（冲突处理）

poc 的冲突**永远不会写进你的工作区**。当提升/压缩/合并无法自动消解时：

1. 命令留下**待办步骤**并退出（exit 1），存储与工作区原封不动；
2. 每个冲突文件生成一份交换文件：`.poc/swap/step/0000__<路径编码>`，内容是标准冲突标记（`<<<<<<< ours` / `=======` / `>>>>>>> theirs`）；
3. 编辑交换文件、去掉冲突标记、把内容改成你想要的结果；
4. `poc --commit` 提交：以交换文件内容回放原操作，一次落盘；或 `poc --cancel` 放弃。

规则：同一时刻至多一个待办步骤，期间新的交互命令会被拒绝（先 commit 或 cancel）；提交前会校验相关栈的 head 没有被移动过，动过就要求 cancel 重来；二进制文件的冲突不可步骤化，直接拒绝。加 `-s` 可以强制任何操作走非交互模式（脚本友好）。`poc -s` 单独运行等于查看当前待办步骤。

---

## 6. 非破坏原则

`poc` 二进制内**不存在销毁**。改变 head 的只有两件事：入栈（记录、提升）和弹出（`-f`）；其余一切操控（压缩、改写、合并、分叉）head 都不变，被替换下来的原 op 一律转入自由池。配合追加式 `poc log`，任何历史形态都能手工还原。

---

## 7. dpoc —— 危险操作（独立程序）

销毁类操作全部隔离在单独的二进制 **dpoc** 中，poc 本体没有销毁代码路径。三层把门：

1. **部署层**：dpoc 是独立二进制，可以不装、不进 PATH；
2. **存储层**：危险模式默认关闭，`dpoc enable` 交互确认（输入 `ENABLE`）后开启，`dpoc disable` 随时关闭；
3. **操作层**：危险动词必须交互终端执行，并完整输入目标 id（如 `DESTROY <64位全哈希>`）；非 TTY 直接拒绝，**没有 `--yes` 旁路**。

动词级许可：`~/.config/poc/dpoc.conf`（或 `$POC_CONFIG_DIR/dpoc.conf`），一行一个 `动词 = allow|deny`；销毁类动词（`opt-destroy` / `compose-remove` / `compose-destroy` / `attic-purge`）默认 deny，`enable` / `disable` / `restore` / `verify` / `audit` / `gc` 默认 allow。

```
dpoc enable | disable
dpoc opt-destroy <ids…>        # 销毁 op → attic（仅限池内 op）
dpoc compose-remove <names…>   # 移除 Compose，成员沉淀自由池
dpoc compose-destroy <names…>  # 移除并连成员进 attic
dpoc restore <ids…>            # 从 attic 找回
dpoc attic-purge <ids…>        # 真删除（不可逆；日志行仍保留并标注）
dpoc gc                        # 全库引用扫描后清理
dpoc verify                    # 哈希重算、引用完整性、审计与日志连续性
dpoc audit                     # 查看审计日志（追加式，永不清除）
```

两段式销毁：destroy 只把对象移入 attic（gc 永不触碰，可 restore），attic-purge 才是真删除。所有 dpoc 动作同样写入 `poc log`。

---

## 8. 四个二进制

| 二进制 | 用途 |
|---|---|
| `poc` | 日常安全操作（本手册第 3–6 节） |
| `dpoc` | 危险操作及安全网（第 7 节） |
| `cpoc` | 链接协作（占位，功能未实现） |
| `fpoc` | poc + dpoc 合集：按第一个词自动分派，可 alias 成 poc 用 |

---

## 9. 常用工作流速查

```console
# 实验一条改动，不满意就整体丢弃
$ poc cmp -N try --fork
$ …改代码… && poc opt -M "try x"
$ poc cmp main                  # 回主线（try 栈保留，随时可再切过去）

# 把另一个栈上的某个变更"捡"到当前栈
$ poc opt <那个栈里的 op id>     # 跨栈提升 = 施加变更（head 延长）

# 把栈顶最近三个 op 揉成一个
$ poc opt <idA> <idB> <idC> -c -M "squash"

# 撤销栈顶（变更进池，不丢）
$ poc opt <栈顶id> -f

# 想看看会发生什么但先不动手
$ poc --dry-run <任意改写命令>
```
