# P.O.C. 1.0 设计文档

**P**roject · **O**peration · **C**ompose —— 用户态、纯本地的版本控制工具，Rust 实现，单二进制。

- 版本：1.0 设计稿 v0.7（供评审微操）
- 日期：2026-10-02
- v0.7 变更（输出改造定稿，详见 docs/OUTPUT-DESIGN.md）：**全端默认英文** + rust-i18n 消息目录（`locales/*.yml` 编译期嵌入，zh-CN 内置，语言链 `POC_LANG → LC_ALL → LC_MESSAGES → LANG → en`）；**anstyle/anstream 主题层**（TTY 保留 ANSI、管道/NO_COLOR/TERM=dumb 自动剥离，minus pager 对 ANSI 透明）；**块式渲染**——小节标题顶格、正文缩进 2 空格、次级信息 4 空格，对齐只基于内容（显示宽度），与终端尺寸无关，箭头一律 ASCII `->`；前端去代数黑话（整体/内部/hunks/共轭不再出现，计数只说 added/deleted/updated）；`--quiet`（只出结果 id 与错误）与 `--verbose`（文件清单、新铸 operation 明细）语义落地；帮助附全局旗标表、程序名随 argv[0]（fpoc 下显示 fpoc）；错误输出统一 `error:`（exit 1）/ `usage:`（exit 2）前缀。**本地化边界**：日志事件的 msg/detail 是持久化数据，落库写英文规范文案；事件 kind 为结构化字段，展示时经目录本地化；op 的消息是用户内容，永不翻译。
- v0.2 变更：CLI 词表整体替换为定稿设计（opt / show / cmp / proj / status / lk + 全局标志）；存储从手写文件迁移到 **redb** 单文件库；新增**自由池**与**步骤协议**两个概念。
- v0.3 变更：`lk --once` 定稿为一次性同步、不注册链接；pager 定稿为内嵌 **minus**（跨平台）；补充实用旗标 `--dry-run` / `--yes` / `--no-sync` / `--quiet` / `--verbose` / `-p` 与命令 `opt --amend`、`show <opt-id>`、`diff`、裸 `lk`；`cmp -N/--fork` 语义在 §5.3 展开。
- v0.4 变更：§9 决策全部按暂定取值定稿；新增**作者信息**：op 载荷增加 author、新增 `poc config` 命令（解析链 = 本仓库 → `$GIT_AUTHOR_*` → Git 全局配置）。
- v0.5 变更：**四二进制族**定稿（poc / dpoc / cpoc / fpoc，核心库复用）；**销毁隔离**定稿——`opt -r` 与 Compose 的删除全部移出 poc，dpoc 承载（三层鉴权 + 动词级许可 + attic 两段式 + 审计）；**链接推迟至 cpoc**（1.0 无 `lk` 命令入口）；新增 §3.7 跨平台注意点。crate 选型表待跨平台复核后另落。
- v0.6 变更：**非破坏原则**定稿（§0 目标 7）——除显式弹出/入栈外，`poc` 操控栈内 opt 不得改变代码库、不得丢失信息（压缩/amend 被替换的原 opt 入池、合并为副本、分叉取前缀）；**O3 修订**为"存在保 head 的入栈落位则入栈，否则复合结果落自由池"；`--fork` 增加可选 op-id（自指定 op 分叉前缀，原栈不动）；`cmp -c` 定稿为**副本语义**（被合并栈保留、当前栈不切换）；新增 **`poc log`**（§5.7）与追加式日志表（§3.3）——按栈记录全部事件，删栈不删史。
- v0.6.1 变更（实现定稿）：**步骤协议全链路落地**（§1.6/§4.4）——置换/嫁接冲突一律进入步骤协议（§2.5"任何一段失败 = 进入步骤协议"优先于 v0.6 的跨度复合自动回退；后者仅保留其结构性情形：含池内/他栈 opt 的压缩恒落自由池）；`--commit` 以交换文件内容回放原操作（graft 链以交换文件内容为准，前置 head 指纹校验），`--cancel` 放弃，待办期间新交互命令拒绝；二进制冲突不可步骤化，直接拒绝。
- §9 决策清单分两层：**已定**（遵你的定稿，不再动）与**待微操**（我推导出的精确语义，等你逐条圈定）。

---

## 0. 1.0 目标与非目标

**目标（全部达成才算 1.0）**

1. **端到端纯本地**：单二进制 `poc`，无网络、无远程协议、无守护进程。
2. **像 Git 一样在本地维护多个代码库的完整历史**：每个代码库 = 一个 Project = 一个含 `.poc/` 的目录。
3. **行级粒度**：内容变更追踪到行；文件级只追踪存在性（新增/删除）。
4. **栈语义历史 + 自由池**：历史是 Compose（有序 Operation 栈）；不入栈的 Operation 进入每项目一个的自由池，仍可按 id 操控。
5. **步骤化交互**：一切交互（编辑器、确认、冲突处理）都可被 `--step/-s` 解构为非交互步骤；冲突经 `.poc/swap` 交换文件解决；**冲突永不污染工作区**。
6. **日常工具永不销毁**：`poc` 二进制内不存在销毁代码路径——opt 的物理销毁与 Compose 的删除（含成员入池的温和形式）均隔离于独立二进制 `dpoc`（§5.9），鉴权单独配置。
7. **信息不破坏（非破坏原则，v0.6）**：除显式**弹出**（`opt -f`）与显式**入栈**（`opt -M` 新铸、`opt <ids>` 提升）外，`poc` 操控栈内 opt 的一切操作——压缩、amend、合并、分叉——不得改变代码库（head 快照不变；副本类命令且不切换当前栈），也不得丢失信息：被替换下来的原 opt 一律转入自由池，原栈原地保留。**Compose 所组合的 Operation 本身也是信息，同样不能丢。**保 head 的落位得不到时（如置换冲突），计算所得的新 opt 改放自由池，栈分毫不动，由用家自行组装。一切栈/opt 变迁记入追加式日志 `poc log`（§5.7）：删栈不删史。

**非目标（1.0 明确不做）**

- 网络协议、加密签名；**链接协作整体推迟至 cpoc**（1.0 无 `lk` 命令入口，§10）。
- 暂存区（index）：`opt -M -i` 直接以 glob 从工作区取变更。
- 重命名检测：重命名 = 删除 + 新增，是合法的整体变更。
- 钩子、子模块、稀疏检出、大文件特殊处理。

---

## 1. 核心概念

### 1.1 Project（proj）

代码库。**身份 = 存储位置的规范路径**。`poc proj` 三种形态：`-N <name>` 在当前目录下新建同名文件夹并初始化；`proj <path>` 在指定目录初始化；`proj` 原地初始化。

### 1.2 Operation（opt）

一次**最小的、自包含的变更**，命令层缩写 **opt**。与 commit 的本质区别：

- **不绑定父节点**：重排会改变前态，父指针无意义。Operation 自包含**前态/后态一对快照**，变更内容由前后快照的行级差分按需导出。
- **变更分两类**，可混在同一次 Operation 里，分解唯一（§2.4）：
  - **内部变更**（乘法）：现存文件的行级内容修改，不管存在性；
  - **整体变更**（加法）：文件存在性翻转（新增/删除），不管内容。

Operation 不可变、内容寻址。差分、逆、共轭一旦计算入库即成为新的 opt 对象（"预先计算集合"按需增长、保持封闭）。op 载荷含**作者**（§5.8 config），类似 git commit 的用户信息。

### 1.3 Compose（cmp）

**有序 Operation 栈**：基态快照（base）+ 自底向上的 opt 序列；全部作用后的快照即头（head）。切换、新建（`-N`；`--fork [op-id]` 自栈顶或栈中指定 op 分叉前缀）、合并（`-c`，产出**副本**、被合并栈保留）；删除不在 `poc`（§5.9 dpoc）。对栈内 opt 的一切非弹出/入栈操控均不破坏信息（§0 目标 7）。

### 1.4 自由池（Free Pool）

**每个项目一个**，收纳不入任何栈的 opt。入池途径：`opt <ids> -f` 从栈弹出、压缩与 `--amend` 被替换下来的原 opt（v0.6）、压缩回退时产出的复合结果、`dpoc compose-remove` 移除 Compose 后其成员沉淀（§5.9）。池内 opt **仍可被 `opt <ids>` 按 id 命中**（从池中提到栈顶、参与压缩等）。自由池是**指针层**概念：opt 对象本身不变，只是归属在栈与池之间移动；`gc` 不清理池内对象。**`poc` 内不存在销毁：唯一的"移除"就是入池。**

### 1.5 Link（cpoc 范畴，1.0 不实现）

> v0.5：链接协作整体推迟至 `cpoc`；`poc`/`dpoc` 1.0 不含链接代码路径与命令入口。以下内容冻结为 cpoc 的设计输入。

Project 间的**活动同步关系**，而非静态引用。字段：name（唯一标识）、targetURI（1.0 = 本机路径，指向另一 Project）、compose 映射（本地名:目标名 多对）、auto 标志。

- 建档即同步；此后**每次执行任何命令前**对 auto 链接自动拉取/推送，保持映射的 Compose 对同态。
- `poc lk <link name>`：手动重同步指定链接。
- `--once/-o`：仅建档时同步一次，之后只手动重同步。
- 同步算法见 §4.5；"同态"指 head 快照一致，跨项目的 opt id 因共轭重写而不同，对应关系由同步映射表维护（§3.3）。

### 1.6 步骤（Step）与交换文件

一切需要交互的操作（冲突合并、危险确认、消息编辑）统一为**步骤协议**：

1. 命令遇到需要交互的点，先**交互式预览**（如冲突涉及哪些 opt id、哪些文件），用家可当场退出放弃；
2. 需要编辑时生成**交换文件**置于 `./.poc/swap/` 下，打开编辑器；
3. `--step/-s` 模式下**不打开任何交互界面**：命令留下待办步骤、一次性输出说明（交换文件路径、下一步指引）后退出；`poc status` 显示当前步骤；用家手工处理后以 **`poc --commit/-c`** 提交步骤结果（继续下一步或完成），或 **`poc --cancel/-q`** 放弃；
4. 步骤是原子的：提交前所有重建只在内存进行，**工作区在步骤期间保持原样**；同一时刻至多一个活动步骤，新交互命令遇待办步骤即拒绝（先 commit 或 cancel）。

### 1.7 与 Git 的概念对照（仅作定位）

| P.O.C. | Git 近似物 | 关键差异 |
|---|---|---|
| Project | repository | 身份 = 路径；无 remote 概念 |
| Operation (opt) | commit | 自包含（pre/post 快照对），无父绑定 |
| Compose (cmp) | branch | 栈语义；重排/合并是一等操作 |
| 自由池 | （近似 stash + reflog） | 一等公民，池内 opt 可随时按 id 回栈 |
| Link (lk) | remote + 自动 pull/push | 保持 Compose 同态的双向同步（cpoc 范畴，1.0 不实现） |
| `poc log` | reflog | 按栈追加、只增不删；删栈不删史，显式 purge 亦留痕（§5.7） |
| 步骤/交换文件 | 冲突标记落盘工作区 | 冲突处理不触碰工作区，走 `.poc/swap` |

---

## 2. 代数模型（设计依据，非运行时组件）

> 本节是对模型的研究，作用有二：论证"历史重写只需一个原语"（§4.2），为 §7.3 提供性质测试 oracle。**运行时不包含任何证明器**——"合法性检查"就是普通的上下文校验，失败即进入步骤协议。

### 2.1 状态与变换

状态 s ∈ S 是文件路径集 F 到（内容 ∪ {⊥}）的映射；快照是状态的显式化。Operation 是 S 上的**偏变换**，由快照对 (pre, post) 给出，pre 即前置上下文校验条件。

### 2.2 整体变更：加法群

存在性翻转构成 (𝔽₂)^F：交换群，每元素自逆。实现细节：删除文件时把内容寄存进 opt，重新添加时取回，翻转才严格可逆。推论：栈中两个整体变更永远可交换。

### 2.3 内部变更：乘法群

行级内容变换，前置上下文不匹配则无定义（偏函数）。封闭性由按需计算维持：差分/逆/共轭计算后入库，opt 集合随之封闭。

### 2.4 差分与分解

- **差分** d(a→b) = a⁻¹b（左除）："撤销 a、再施加 b"的复合变换，不是普通 diff；应用上下文 = a 的终态。
- 任一 opt 的前后快照差分唯一分解为整体部分 + 内部部分。
- 两群合成是带偏作用的半直积，一般不是环；局部分配律恰为下述交换判据。

### 2.5 合法性判据（全部化为工程检查）

- 内部 op 与整体 op 可交换 ⇔ 文件支持集不相交；
- 两个整体 op 恒可交换；
- 两个内部 op 可交换 ⇔ 行级三路合并无冲突；
- **栈重排合法 ⇔ 所需交换全部存在；任何一段失败 = 进入步骤协议（§1.6），不落地**。

### 2.6 代数如何让代码紧致

- opt 自包含（pre/post），**逆 = 前后互换**；
- 提升/压缩/合并/同步全部是同一个 graft 原语的组合（§4.2）；
- 指针层（栈序、自由池归属、链接映射）完全在代数之外——这正是"静态对象库 + 可变指针"的分离；
- **非破坏的代数依据（v0.6）**：压缩/合并/重排只改变复合的**分解方式**，不改变复合总值（head 快照）；head 的改变仅有入栈（复合延长）与弹出（复合缩短）两个显式原语。"信息不破坏"由此是"不可变对象 + 指针移动"的推论，无需额外机制。
- 性质测试 oracle：P1 重排不变式、P2 逆复合还原、P3 压缩快照不变、P4 记录幂等、P6 非破坏不变式、P7 日志完整（§7.3）。

---

## 3. 数据模型与存储（redb）

### 3.1 目录布局

```
<project>/
  .poc/
    store              # redb 单文件：全部对象 + 全部指针层状态
    swap/              # 步骤交换文件（会话性，可随时清空）
  .pocignore           # 可选，忽略规则
  （工作区文件）
```

无 TOML、无栈文件、无锁文件：可变状态全部在 DB 里，由事务保证原子性。

### 3.2 对象与编址

哈希：BLAKE3-256，十六进制；对象 id = H(类型标签 ‖ 规范编码)。**可哈希载荷**用手写长度前缀二进制（规范确定），持久化由 redb 承担——编码只服务于编址，不再承担存储格式职责。三类不可变对象：

- **blob**：文件原始字节。mode ∈ {regular, exec, symlink}；符号链接内容 = 目标路径字节。
- **tree（快照）**：扁平排序表（路径按字节序 × mode × blob 哈希），不建子树对象。
- **op**：pre_tree_hash ‖ post_tree_hash ‖ author(name ‖ email) ‖ msg(UTF-8) ‖ time_ms(u64)。author/msg/time 进入身份：改写任一 = 产生新对象，栈引用随之更新，重写成本低。

### 3.3 redb 表结构

```
blobs:    [u8;32] → bytes                    # 静态：写后不改
trees:    [u8;32] → tree 编码                # 静态：写后不改
ops:      [u8;32] → op 编码                  # 静态：写后不改
composes: name → (base, head, Vec<op_id>)   # 指针层：写事务更新
pool:     op_id → 入池时间                    # 指针层：自由池成员
attic:    op_id → (原归属, 入 attic 时间)     # 指针层：dpoc 销毁隔离区，gc 永不触碰
audit:    seq → (verb, 对象ids, 时间)          # 指针层：dpoc 追加式审计，永不清除
log:      seq(u64) → 事件编码                  # 指针层：追加式操作日志（§5.7），只增不改不删
meta:     key → value                        # current、项目名、user.name/user.email、dp.enabled、活动步骤、log 序号
```
（links / syncmap 两张表随链接推迟至 cpoc，1.0 不建。）

静态/指针分离同构于"Operation 不可变 / Compose 可变"：前三张表是静态对象库，其余六张是可变指针（attic/audit 仅 `dpoc` 写入；log 由 poc 与 dpoc 共同追加），全部变更走写事务。

### 3.4 事务模型

- 每条命令 = **只读快照上重建验证 + 至多一个写事务提交**；失败无副作用。
- 单写者由 redb 自身保证（第二个写进程打开即报错），不再手写锁文件。
- 栈操作、日志追加（`poc log` 的事件行与引发它的变更**同一事务**）、链接同步、步骤提交，全部同一模式。

### 3.5 工作区规则

- 无暂存区；`opt -M` 比较工作区与 head，`-i` 以 glob 限定入选文件。
- `.pocignore`：glob 规则；空目录不追踪。
- 字节原样存储，无换行符改写；二进制文件的行级差分自然退化为整文件替换。

### 3.6 崩溃与恢复

- redb 自带崩溃安全；对象只增不改，指针行事务更新。
- `swap/` 是会话性文件，可随时删除重建；活动步骤记录在 meta 中，指向其交换文件。
- `poc -o`（gc）：清除不被任何 compose、自由池、attic、活动步骤引用的对象；log 表不在清理范围（追加式流水，§5.7）。非破坏原则（§0 目标 7）下，opt 离栈只有入池/入 attic 两条去路，gc 实际只兜底异常残留。

### 3.7 跨平台

- 依赖全部为纯 Rust 或 crossterm 系，Linux / macOS / Windows 三平台可用（选型复核见 §7.2 与评审讨论）。
- 路径一律 `PathBuf` + `fs::canonicalize`（Project 身份 = 规范路径），无硬编码分隔符；字节原样存储、不改换行符，天然平台中立。
- 平台差异点：① 可执行位 Windows 无对应——mode 保真存储，不可表示时 `status` 告警；② 符号链接在 Windows 需特权/开发者模式——缺权限时按普通文件记录目标内容并告警；③ 编辑器回退链 `$POC_EDITOR` → `$VISUAL` → `$EDITOR` → Unix `vi` / Windows `notepad`；④ 时区展示取系统 tzdb，不可用退化为 UTC 标记。

---

## 4. 核心算法

### 4.1 快照差分（行级）

- tree 层：路径集对称差 → 整体部分；共同路径但 blob 不同 → 内部部分。
- 行级：Myers（经 `similar` crate，隔离在 `diff` 模块后可替换）。输出：hunk 列表。

### 4.2 graft：唯一的重写原语

**graft((X→Y), Z) → W**：把"X→Y 的变更"施加到快照 Z 上 = 行级三路合并 merge3(base=X, ours=Z, theirs=Y)。无冲突 → W；有冲突 → 报告文件与冲突 hunk，转入步骤协议。

### 4.3 命令 → graft 组合

**通用前置规则**：改写栈或切换栈的命令（`opt <ids>` 各形态、`cmp` 切换/合并）要求工作区干净（head == 工作区快照）；`opt -M`（记录本身消化变更）与只读命令不要求。全部操作先内存重建、后单事务落盘。

**非破坏原则（v0.6，§0 目标 7）**：改变 head 的显式原语只有**入栈**（`opt -M`、`opt <ids>` 提升）与**弹出**（`opt -f`）；其余一切栈操控（压缩、amend、合并、分叉）执行前后 head 快照不变——副本类命令（`cmp -c`）且不切换当前栈。被替换下来的 opt 一律转入自由池或原地保留，`poc` 内 opt 只在栈与池（及 dpoc 的 attic）之间流动，无第三条去路；全部变迁按序写入追加式日志（§5.7），删栈不删史。

| 命令 | 语义 | 合法性 |
|---|---|---|
| `opt -M [-i glob]` | 工作区 Δ（glob 限定）→ 新 opt 压入当前栈 | 恒合法（非空时） |
| `opt <ids>` | 提升重排：按给定顺序把命中 id 的 opt（栈内或池内）逐一提到栈顶，被顶替者保留下移 | 逐个 graft，任一冲突 → 步骤 |
| `opt <ids> -f` | **显式弹出**：命中 opt 出栈沉淀自由池，head 回退重算 | 需工作区干净；恒合法 |
| `opt <ids> -c` | 压缩（≥2 个）为一个新 opt c：命中者恰为当前栈**连续段** → 精确复合**原位替换**（head 不变，原 opts 入池）；否则 **c 落自由池**、栈分毫不动（§5.1） | 连续段恒合法；聚拢冲突 → 步骤协议（§1.6） |
| `opt <ids> -r` | **销毁** opt 对象（栈内者先出栈重建），不可逆 | 步骤协议确认（§1.6） |
| `opt --amend` | 以（栈顶.pre, 工作区快照）改写栈顶，并入工作区变更；**被替换的原栈顶 opt 转入自由池**（v0.6 补记） | 复合恒合法；head 树不变 ⇒ 工作区零差异 |
| `cmp <name>` | 切换当前 Compose，物化其 head | 需工作区干净；恒合法 |
| `cmp -N [--fork [op-id]]` | 新建：base = 当前 head、空栈；`--fork` 缺省自栈顶复刻整栈，`--fork <op-id>` 复刻自底到该 op 的**前缀**（该 op 为新栈顶，原栈不动，§5.3） | 恒合法；op-id 非栈顶时切换要求工作区干净 |
| `cmp <names> -c` | 合并多个 Compose 为**一个新栈（副本）**（栈构造见 O2，参照 git merge 的三路思想）；被合并栈保留、当前栈不切换（§5.3） | graft 链可冲突 → 步骤 |
| `poc -o` | gc：清除未引用对象 | 恒合法 |
| `poc log` | 追加式操作日志的只读视图（§5.7）；事件写入与各变更命令同事务 | 恒合法 |

Compose 的删除不在 `poc`——温和形式（成员入池）与销毁形式都在 `dpoc`（§5.9）；O12 的"移除最后一个 Compose"问题随之消失（`poc` 中 Compose 只增不删）。非破坏原则下，`poc` 对栈的全部重组（压缩/合并/分叉/amend）都可由"指针移动 + 原 opt 入池"论证：任何一步都能从日志与自由池手工还原。

### 4.4 步骤协议（运行时形态）

- 步骤记录：meta 中存（步骤种类、涉及的 opt ids、交换文件清单、前置状态指纹）；交换文件在 `.poc/swap/<step>/` 下按路径编码命名。
- **冲突交换文件**：每冲突文件一份，内容为冲突标记文本（<<<<<<< / >>>>>>> 包裹双方段落）；`--commit` 时解析已编辑文件作为合并结果，重新执行被解构的操作（graft 链以交换文件内容为准）。
- **危险确认**：`poc` 1.0 无危险动词（销毁族在 `dpoc` §5.9，自带确认协议；`lk` 已推迟），确认式交互暂无使用点；`--yes` 保留为未来确认类旗标。
- `-s` 模式输出契约：一次输出"发生了什么 + 交换文件路径 + 下一条确切命令"，退出码 1；无待办时 `poc -s` 等价 `poc status` 的步骤节。

### 4.5 链接同步流程（cpoc 范畴，1.0 不实现；保留作设计输入）

1. 命令执行前，对每个 auto 链接（或 `poc lk <name>` 指定链接）逐映射对处理（全局 `--no-sync` 可单次跳过）；
2. 读目标 Project 的 store（只读）→ 找两栈的公共祖先快照（公共前缀）；
3. **拉取**：目标独有 opt 逐个 graft 到本地栈顶（顺序 = 目标栈序）；**推送**：本地独有 opt 逐个 graft 到目标栈（对目标 store 开写事务）；同步映射表（syncmap）登记 id 对应；
4. 收敛序 = 公共前缀 + 本地独有 + 目标独有（确定性，见 O15）；
5. 任一 graft 冲突 → 同步中止、留下步骤，**本次命令不执行**（退出码 1）。

---

## 5. CLI 规范

### 5.0 语法与全局标志

```
poc [全局标志] [命令 [参数…]]
```

- **标志归属规则**：命令词（opt/show/cmp/proj/status/diff/config/log）之前的旗标是全局的，之后的同名短旗标归命令。因此 `-c` 在 `poc -c`（提交步骤）与 `poc opt id -c`（压缩）、`poc cmp a b -c`（合并）、`poc show -c`（Composes 节）、`poc lk -c`（映射）中含义互不相同，由位置唯一确定。
- 全局标志：
  - `--gc/-o`：执行命令前先 gc 优化；
  - `--step/-s`：不打开任何交互界面，解构为步骤；
  - `--commit/-c`：提交当前步骤结果，进入下一步；
  - `--cancel/-q`：放弃当前步骤；
  - `--dry-run`（v0.3 补充）：预演——完整执行内存重建与合法性验证但不落盘，输出将新建/改写哪些 opt（含真实 id）、将写哪些表、有无冲突。重建本就在内存进行，dry-run 近乎零成本；
  - `--yes/-y`（v0.3 补充）：跳过确认（1.0 中预留——`poc` 无危险动词，销毁类确认在 `dpoc` 且不受此旗标影响）；`-s` 模式下等价"立即确认、不建步骤"；
  - `--no-pager/-p`（v0.3 补充，自 show 提升为全局）：本次所有输出不分页；
  - `--quiet` / `--verbose`（v0.3 补充）：只输出关键行（id、错误）/ 额外输出 graft 链逐步细节。
- `<any command>` 可以为空：`poc -o` 仅优化；`poc -s` 输出当前步骤；`poc -c` 仅提交；`poc -q` 仅取消；裸 `poc` 输出帮助。

### 5.1 `opt` —— Operation 的创建与操控

```
poc opt -M "message" [-i "filename glob"]
```
在当前 Compose 创建一个 Operation 并压入栈顶——显式**入栈**原语之一。`-M` 给出消息，缺省打开编辑器编辑消息；`-i` 以 glob 匹配入选文件，缺省全部差分。

```
poc opt <operation ids> [-f] [-c]
```
以 id（≥4 位十六进制唯一前缀，可命中栈内或自由池内 opt）操控既有 opt：

- **缺省（提升）**：栈顶依给定顺序变为这些 id 的 opt——即把栈中/池中的这些 opt 按序提到栈顶，被顶替者下移保留。逐个 graft，冲突 → 步骤。（显式**入栈**原语之二。）
- **`-f/--free`**：将命中 opt 弹出到自由池，head 回退重算。（显式**弹出**原语。）
- **`-c/--compact`**（v0.6 修订 O3）：把命中 opt（≥2 个）压缩为一个新 opt c。**落位规则**：命中者恰为当前栈上的**连续段** → c = 跨度精确复合（pre = 段首.pre、post = 段末.post），**原位替换该段**——head 快照不变、工作区零差异，被压缩的原 opt 全部转入自由池，用家可随时重组；其余情形（选择不连续、含池内 opt）→ **不存在保 head 的入栈落位，c 直接落入自由池**：栈与工作区分毫不动、原 opt 原地保留，是否入栈、入哪个栈由用家显式决定（`opt <c-id>`，即入栈原语）。非连续/跨上下文的复合经 graft 链（M3）；聚拢冲突 → 步骤协议（§1.6，v0.6.1）。
- （v0.5：`-r/--remove` 移除——opt 物理销毁只在 `dpoc`（§5.9）；`poc` 内的"移除"只有 `-f` 入池。）

```
poc opt --amend [-M "message"]    （v0.3 补充）
```
改写栈顶：以（栈顶.pre, 工作区快照）替换栈顶 opt，消息取 `-M` 或编辑器。工作区变更被并入栈顶，等价"弹出栈顶 + 重新记录"的原子形式，复合恒合法。**被替换的原栈顶 opt 转入自由池**（v0.6，非破坏原则）；只改消息时 head 树不变，工作区零差异。

### 5.2 `show` —— 状态总览

```
poc show [--composes|-c] [--operations|-o]
poc show <opt-id>                 （v0.3 补充）
```
无参数：展示当前 Compose 名、Operations 栈（ASCII 字符画，栈顶在上）。两个节选旗标缩小展示面（与 `<opt-id>` 形态互斥；`--links` 随链接推迟至 cpoc）。输出超出终端窗口时交给内嵌 pager（minus，§7.2），全局 `-p` 禁用。`show <opt-id>`：单 opt 详情——元数据（作者、时间）+ 行级差异全文。

### 5.3 `cmp` —— Compose 管理

```
poc cmp <compose names> [--compact|-c] [-N "结果名"]
```
恰一个 name：切换当前 Compose。`-c`（v0.6 定稿）：把多个 Compose 合并为**一个新建 Compose（副本）**——构造按 O2（公共祖先快照 + 各栈独有段逐个 graft，参照 git merge 三路思想），**被合并的栈原样保留、当前栈不切换**，工作区零差异；结果栈名由 `-N` 给出，缺省 `merged`（重名自动追加序号）。graft 冲突 → 步骤，整体不落地。要看合并结果，显式 `poc cmp <结果名>`（一次普通切换）。"合并后删除源栈"不是 `poc` 的动作——先 `-c` 得副本，源栈的去留交给 `dpoc`（§5.9）。**（v0.5：`-r/--remove` 移除——Compose 的删除（含成员入池的温和形式）不在 `poc`，见 §5.9 dpoc。）**

```
poc cmp -N "compose name" [--fork [op-id]]
```
新建 Compose。Compose 由 **base（起点快照）与栈序列**两要素定义，head = base 复合整栈；各形态的差别就在继承什么：

- **非 fork**：base = 当前 head、空栈。语义 = **"从当前状态出发、历史从零算"**。新栈的 head == 其 base == 当前 head，切换过去工作区内容不变；此后两边各自生长、互不引用。Git 类比：`git checkout --orphan` 且以当前内容为起点。适合实验线、重构试错。（若要彻底白手起家——base = 空快照、首次记录把全部现有文件记为整体变更 +N 文件——见 O4 备选。）
- **`--fork`（无 op-id，缺省）**：以当前**栈顶**为分叉点 = 复刻整栈。base = 当前 base、复刻全部 opt（共享 opt 对象，不可变、共享无副作用）。语义 = **"连历史一起复刻"**。Git 类比：`git checkout -b`。
- **`--fork <op-id>`**（v0.6 新增）：op-id 须命中**当前栈上**的 opt；新栈 = 自底到该 opt 的**前缀**，该 opt 即新栈顶，其余 opt 留在原栈——**原栈分毫不动**（非破坏原则）；opt 对象共享。创建后切换到新栈：op-id 为栈顶时工作区零差异；否则物化该 op 的 post（等同一次显式切换，要求工作区干净）。

一句话：不带 `--fork` = 新历史；`--fork` = 平行历史（缺省整栈、可指定分叉点）。各形态切换时都要求工作区干净。

### 5.4 `proj` —— Project 管理

```
poc proj -N "project name"     # 当前目录下新建同名文件夹并初始化
poc proj path/to/project       # 在指定目录初始化
poc proj                       # 原地初始化
```

### 5.5 `status`

自由池中的 opt、当前未铸成 Operation 的变更量（整体 +/- 文件、内部逐文件 hunk 统计）、当前待办步骤（`-s` 指示）。超窗口交给内嵌 pager。

### 5.6 `diff`（v0.3 补充）

```
poc diff [<a> [<b>]] [--stat]
```
无参数：工作区 vs 当前 head 的行级差异全文（未铸成 Operation 的变更）；一个 opt id：该 opt 的差异；两个 id：两快照间的差异。`--stat` 只输出逐文件统计。分页同 §5.2；与 `show <opt-id>` 的重叠是有意的——show 给详情，diff 给纯比较。

### 5.7 `log`（v0.6 新增）

```
poc log [<compose 名>] [--all|-a]
```

**追加式操作日志**：按栈记录一切改变栈/池/opt 归属的事件——建栈（proj 初始化、`cmp -N`、`--fork`）、入栈（`opt -M` 新铸、提升）、弹出（`-f`）、压缩（含回退入池）、`--amend`、合并（`cmp -c`），以及 `dpoc` 的 compose-remove / compose-destroy / opt-destroy / restore / attic-purge。每行含：序号、时间、事件种类、栈名、opt id、作者与消息快照。

- **只增不改不删**：log 表不在 gc 范围（§3.6）；**栈被删除，其日志仍在**（删栈不删史）；op 被 `attic-purge` 真删除后，日志行保留并标注"已清除"——信息可逆到显式清除为止，痕迹永久。
- 视图：缺省展示当前栈的编年史（旧 → 新）；给定栈名回放任意栈（含已删除的栈）；`-a` 按栈分组展示全部。超窗交给内嵌 pager。
- 事件与引发它的业务变更**同一写事务**落盘（§3.4）：要么都有、要么都没有。
- 定位：`show` 看现状（栈与池此刻的形态），`log` 看流变（每个栈如何走到今天、消失的栈去过哪里）。audit（§5.9）面向安全追责，log 面向历史重组——配合自由池与 attic，任何历史形态都可依日志手工还原（§0 目标 7 的"信息可逆"）。

### 5.8 `config`（v0.4 新增）

```
poc config                       # 列出生效值及其来源
poc config user.name "名字"       # 写入本仓库（redb meta）
poc config user.email "a@b.c"
poc config --unset user.name
```
作者信息在 `opt -M` 落库时解析，顺序：本仓库 meta → `$GIT_AUTHOR_NAME` / `$GIT_AUTHOR_EMAIL` → Git 全局配置（`~/.gitconfig` 的 user 节，含 include 展开）→ 报错并提示设置。name 必有；email 可空（展示时省略）。

### 5.9 `dpoc`（v0.5 新增，独立二进制）

`poc` 的结构不变量：**日常工具永不销毁**——`poc` 二进制内不存在销毁代码路径，opt 的物理销毁与 Compose 的删除（含成员入池的温和形式）均不在 `poc`。全部危险操作隔离到独立二进制 **`dpoc`**（只进行危险操作及其安全网），共用核心库，可单独编译、单独安装、单独授权。由此 `poc` 的每个操作都可由"指针移动 + 不可变对象"论证可逆性。

**三层鉴权**：
1. **部署层**：独立二进制，可单独安装/授权（不进日常 PATH 亦可用）；
2. **存储层**：meta `dp.enabled` 默认 `false`；`dpoc enable` 交互确认后开启，`disable` 随时关闭；
3. **操作层**：危险动词必须 TTY 交互并完整输入目标 id（如 `DESTROY <64位全哈希>`），非 TTY 直接拒绝——防脚本误触，**不设 `--yes` 旁路**。

**动词级许可**：`~/.config/poc/dpoc.conf`，手写极简格式（一行 `verb = allow|deny`，解析器约 30 行、零依赖）；销毁类动词默认 `deny`、未列出即拒绝；`enable/disable/restore/verify/audit` 默认 allow。

**两段式销毁（attic 隔离区）**：`opt-destroy` / `compose-destroy` 只把对象移入 attic 表（`gc` 永不触碰、可 `restore` 找回）；`attic-purge` 才是真删除（同样要求完整输入 id）。

```
dpoc enable | disable
dpoc opt-destroy <ids…>           # 销毁 opt → attic
dpoc compose-remove <names…>      # 移除 Compose，成员沉淀自由池（原 cmp -r 的温和形式）
dpoc compose-destroy <names…>     # 移除并连成员销毁 → attic
dpoc restore <ids…>               # 从 attic 找回
dpoc attic-purge <ids…>           # 真删除（不可逆）
dpoc gc --deep                    # 全库引用扫描后清理（poc -o 仅清不可达）
dpoc verify [--cross-refs <path…>]  # 哈希重算、引用完整性、审计与 log 连续性
dpoc audit                        # 查看审计日志（追加式，永不清除）
```

**审计**：audit 表追加记录 verb / 对象 ids / 时间，永不清除，`verify` 检查连续性。销毁前若对象可能被其他项目引用，给出显式警告（深检查由 `verify` 承担）。

**销毁与日志（v0.6）**：dpoc 的移除/销毁/恢复/清除同样写入 `poc log`（§5.7）——销毁改变栈与池的形态，就必须留痕。`attic-purge` 之后对象真删除（不可逆），但日志行保留并标注"已清除"：**内容可逆到显式清除为止，痕迹永久**。audit 面向安全追责（动词、时间），log 面向历史重组（栈、op、消息），二者互补、都只增不删。

### 5.10 示例会话

```
$ poc proj -N demo
$ cd demo
$ poc config user.name "你的名字"
$ poc config user.email "you@example.com"
$ echo "# demo" > README.md
$ poc opt -M "init readme"
  1 added
  created operation 3f9c0a1b "init readme"
$ poc cmp -N experiment          # 空栈新 Compose，base = 当前 head
$ poc cmp -N try-x --fork        # 分叉自栈顶 = 复刻当前整栈
$ poc cmp -N back --fork 3f9c    # 分叉自 3f9c：它成为新栈顶，原栈不动
$ echo "more" >> README.md
$ poc opt -M "try x" -i "README.md"
$ poc opt 3f9c                   # 提升栈底 opt 到栈顶（显式入栈；代数合法时）
$ poc opt ab12 -f                # 弹出：ab12 入自由池（显式弹出原语）
$ poc opt 3f9c ab12 -c           # 压缩（ab12 在池中）：复合 c 落自由池，栈分毫不动
$ poc opt --amend -M "try x v2"  # 改写栈顶并并入工作区变更（原 opt 入池）
$ poc log                        # 当前栈全部事件（旧 → 新）
$ poc log -a                     # 全部栈，含已删除的栈
$ poc --dry-run opt 3f9c         # 预演：将新建哪些共轭 opt、是否冲突
$ poc diff --stat                # 未记录变更的逐文件统计
$ dpoc opt-destroy ab12           # 危险操作走 dpoc（进 attic，可 restore；log 留痕）
$ poc -s opt 3f9c                # 非交互模式：冲突则留下步骤
$ poc status                     # 查看步骤与交换文件指引
$ （编辑 .poc/swap/…）
$ poc --commit                   # 提交步骤结果
```

（v0.7 起终端输出默认英文，`POC_LANG=zh-CN` 可切回中文；输出形态见 docs/OUTPUT-DESIGN.md §4。）

---

## 6. 错误处理与诊断

- 错误分类：`NotAProject` / `DirtyWorkingTree` / `StepPending`（已有待办步骤）/ `Conflict{files, hunks}` / `NotFound(ref)` / `StoreLocked`（另一写进程持有 redb）。`dpoc` 另有 `DangerDisabled` / `VerbDenied{verb}` / `NotATTY` / `ConfirmMismatch`。
- **冲突永不落地**：重建全在内存；拒绝时工作区与 DB 均保持原样，经交换文件（§4.4）解决后一次性提交。
- **日志与变更同生死（v0.6）**：log 事件行与引发它的业务变更同事务提交（§3.4），不存在"变更落地而日志缺失"的状态。日志只增不改不删；`NotFound` 对已删除的栈名不生效于 log——`poc log <已删栈名>` 仍可完整回放其历史。
- 退出码：0 成功；1 拒绝（冲突 / 脏工作区 / 待办步骤 / 同步失败）；2 用法错误。

---

## 7. 工程结构

### 7.1 Crate 与模块

```
poc/
  Cargo.toml          # [lib] + [[bin]] poc / dpoc / cpoc / fpoc
  docs/DESIGN.md
  src/
    lib.rs            # 核心库：全部业务逻辑，四个二进制共用（"poc 本身的逻辑要能复用"）
    bin/poc.rs        # 安全本地操作（1.0 交付）
    bin/dpoc.rs       # 危险操作（1.0 交付，§5.9）
    bin/cpoc.rs       # 协作操作（链接推迟，1.0 仅占位）
    bin/fpoc.rs       # 全功能合集 = poc + dpoc + cpoc（想用的人 alias 成 poc）
    hash.rs  object.rs  db.rs         # 编址、对象、redb 表与事务（含 attic/audit/log）
    tree.rs  snapshot.rs              # 快照构建 / 工作区扫描
    diff.rs  merge.rs                 # 行级差分、graft 三路合并
    compose.rs  pool.rs  step.rs  log.rs   # 栈操作、自由池、步骤协议、追加式操作日志
    config.rs                         # 作者信息与 config 命令
    ignore.rs  gc.rs  pager.rs  render.rs   # 忽略规则、gc、minus 封装、ASCII 渲染
```

### 7.2 依赖（刻意最少）

| 组件 | 选定 | 备选（落选理由） | 备注 |
|---|---|---|---|
| CLI | `clap` v4（derive） | bpaf | 事实标准；全局旗标按位置手工预剥离后再进 clap，保证 §5.0 消歧规则 |
| 存储 | `redb` | heed/LMDB（C FFI）、fjall（读放大）、sled（维护停滞）、RocksDB（重） | 纯 Rust、单文件、ACID、跨平台 |
| 哈希 | `blake3` | sha2 | 纯 Rust 内核 + 可选 SIMD；`hash.rs` 隔离可换 |
| 行差分 + 三路合并 | `similar`（含 `similar::merge`） | imara-diff（更快但无合并）、手写（diff3 正确性风险） | 一石二鸟：graft 只需封装 |
| 遍历 + ignore 语义 | `ignore`（ripgrep 系） | 手写 walkdir + 匹配（边界多） | `add_custom_ignore_filename(".pocignore")`、并行遍历；`globset` 为其配套，`-i` 参数继续用 |
| pager | `minus` | 外部 less/$PAGER（Windows 无）、手写 | 内嵌跨平台；M1 接入 |
| 终端输出 | `anstream` + `anstyle` | colored / owo-colors（无自动降级） | NO_COLOR / TTY 检测 / 管道去 ANSI |
| Git 全局配置 | `gix-config` | shell 调 git（破坏单二进制自足）、git2（C FFI 太重） | 纯 Rust 解析 ~/.gitconfig 含 include |
| 目录定位 | `dirs` | home | `config_dir` 定位 dpoc.conf 等 |
| 时间展示 | `jiff` | chrono / time / humantime | 纯 Rust、现代 API，仅展示 |
| 错误 | `thiserror` | anyhow、snafu | 库层枚举错误 |

dev 依赖：`proptest`（性质测试 P1–P5）、`assert_cmd` + `predicates` + `tempfile`（CLI 黄金测试）、`insta`（可选，ASCII 输出快照）。

刻意不引入：`serde`/`serde_json`（可哈希载荷手写规范编码）、`tokio`（全同步 I/O，minus 用阻塞模式）、`git2`/完整 `gix`（只取 gix-config）、`walkdir`（被 ignore 覆盖）、`hex`（自写编解码）。

全部依赖均为纯 Rust 或 crossterm 系，Linux / macOS / Windows 三平台可用（§3.7）。编辑器走 `$POC_EDITOR`→`$VISUAL`→`$EDITOR`→平台缺省：Unix `vi` / Windows `notepad`。

### 7.3 测试策略

- 单元：diff / merge 固定用例；对象编解码往返；redb 表读写。
- **性质测试（代数付账处）**（已落地：`src/properties.rs`，proptest 每条 64 例随机锤打；P5 随链接推迟至 cpoc 后实现）：
  - P1 重排不变式：合法提升后 head 快照不变；
  - P2 逆复合：opt 后接其逆 = base 快照；
  - P3 压缩不变式：`-c` 后 head 快照不变；
  - P4 记录幂等：`opt -M` 后工作区快照 == head；
  - P5 同步收敛：lk 同步后映射对 head 快照一致（随链接推迟至 cpoc）。
  - P6 非破坏不变式（v0.6）：压缩/amend/合并/分叉执行前后 head 快照不变，且 `cmp -c` 不切换当前栈、原 opt 不丢失（入池或原地保留）；
  - P7 日志完整（v0.6）：任一变更命令后 log 恰增对应事件行；删栈、attic-purge 之后历史行全部保留。
- 端到端：黄金文件 CLI 测试，含 `-s` 步骤协议全链路（建步骤 → 编辑交换文件 → --commit / -q）。
- 作者解析链：本仓库 meta → `$GIT_AUTHOR_*` → Git 全局配置 → 报错，各分支单测（临时目录注入假 HOME）。
- dpoc 拒绝链：`dp.enabled=false` / 非 TTY / 动词 deny 各分支；attic 两段式（destroy → restore 还原如初 → purge 后不可恢复）；审计连续性。

---

## 8. 里程碑

- **M0 地基**：redb store、`proj` 三形态、`opt -M`（整文件级）、`config` 与作者解析、`show`、`status`（含自由池表）——端到端可用。
- **M1 行级**：diff 与 graft 三路合并；`-i` glob；show 的 ASCII 栈图与内嵌 pager（minus）。
- **M2 栈操控与步骤**：`opt <ids>` 提升 / `-f` / `-c`（连续段压缩 + 自由池回退）/ `--amend`（原 opt 入池）；`cmp -N` / `--fork [op-id]` / 切换；`log` 命令与日志表；步骤协议与交换文件全链路。
- **M3 合并与 dpoc**：`cmp -c` 合并（副本语义）；置换聚拢的跨段/池内压缩；`dpoc` 全动词（三层鉴权、动词级许可、attic/restore/purge、审计、verify）。
- **M4 收尾**：`-o` gc、fpoc 合集与四二进制打包、错误文案、文档。

1.0 验收 = M0–M4 全部完成，且 §0 七条目标逐条可演示。

---

## 9. 决策清单

### 9.1 已定（遵你的定稿，不再动）

CLI 词表 = §5 全部（opt / show / cmp / proj / status / lk 及旗标）；存储 = redb 单文件；自由池概念；步骤协议与 `.poc/swap`；链接 = 活动同步关系；冲突走交换文件而非工作区标记；`lk --once` = 只同步一次、**不注册链接**（`-n` 仅建档时必填）；pager = 内嵌 **minus**（跨平台，不依赖外部 less）。

**2026-10-01 评审**：§9.2 的 O1–O20 **全部按表中暂定取值定稿**（并入本节生效）。

**v0.4 新增定稿**：**作者信息**——op 载荷新增 author(name ‖ email) 并进入对象身份；新增 `poc config` 命令管理本仓库用户信息；解析链 = 本仓库 meta → `$GIT_AUTHOR_NAME`/`$GIT_AUTHOR_EMAIL` → Git 全局配置 → 报错提示；name 必有、email 可空。

**v0.5 新增定稿**：**四二进制族**——`poc`（安全本地）/ `dpoc`（只进行危险操作及其安全网）/ `cpoc`（协作，1.0 占位）/ `fpoc`（全功能合集，可 alias 成 poc），四者独立可用、业务逻辑全部在核心库复用；**销毁隔离**——`opt -r`、`cmp -r` 自 poc 移除，opt 物理销毁与 Compose 删除（含温和形式）只在 `dpoc`（三层鉴权 + 动词级许可 + attic 两段式 + 审计，§5.9）；**链接推迟至 cpoc**（1.0 无命令入口，§1.5/§4.5 冻结为设计输入）。crate 选型表（§7.2 扩充）待跨平台复核讨论后另落。

**v0.6 新增定稿（2026-10-02）**：**非破坏原则**（§0 目标 7）——除显式弹出（`opt -f`）与显式入栈（`opt -M`、提升）外，`poc` 操控栈内 opt 不得改变代码库、不得丢失信息；Compose 所组合的 Operation 也是信息。具体化：压缩/amend 被替换的原 opt 一律转入自由池；`cmp -c` 产出**副本**、被合并栈保留、当前栈不切换；`--fork` 取前缀、原栈不动；一切变迁入追加式日志，删栈不删史。**`poc log` 新增**（§5.7；log 表 §3.3）：按栈记录建栈/入栈/弹出/压缩/amend/合并/分叉及 dpoc 的移除/销毁/恢复/清除，事件与业务变更同事务，gc 与 purge 不触碰日志（purge 后行保留、标注已清除）。**`--fork [op-id]`**：缺省 = 自栈顶分叉（即整栈复刻）；带 op-id = 复刻自底到该 op 的前缀、该 op 为新栈顶、其余留原栈、创建后切换（非栈顶时物化、要求工作区干净）。**`cmp -c`**：结果为新建 Compose，名由 `-N` 给出、缺省 `merged`（重名追加序号）；O2 构造不变。**O3 修订**：压缩落位由"最低槽位/落栈顶"改为"存在保 head 的入栈落位（连续段原位替换）则入栈，否则复合结果落自由池"；回退产物 = 跨度复合（含被跨越的中间 op，忠实于"压缩所覆盖的跨度"）。落位/回退的精确语义由我按指令推导，可微操。

### 9.2 历史清单（2026-10-01 评审：O1–O20 全部按暂定取值定稿，保留备查）

> O7、O13 更早定稿移入 9.1，编号空缺以便与 v0.2 对照；其余各项均按表中"暂定取值"列生效。O2 / O6 / O14 / O15（链接相关）随链接推迟至 cpoc 而冻结。

| # | 决策点 | 暂定取值 | 备选 |
|---|---|---|---|
| O1 | 全局 `-c`（commit）与命令 `-c`（compact）同字母 | 按位置消歧：命令词前 = 全局 | 改全局 commit 为 `--commit` 无短旗标 |
| O2 | `cmp -c` 合并的栈构造 | 公共祖先快照 + A 独有段 + B 独有段逐个 graft（三路思想） | 以最长栈为骨干，另一栈 graft 上去 |
| O3 | `opt -c` 压缩结果落位 | 被选栈位中最低者的槽位；全为池内 opt 则落栈顶 | 恒落栈顶 |
| O4 | `cmp -N` / `--fork` 的基态 | 非 fork：base = 当前 head、空栈——**新历史**，起点 = 当前状态（类比 orphan 分支保留内容）；fork：base = 当前 base、复刻整栈——**平行历史**（类比 checkout -b）。展开解释见 §5.3 | 非 fork 改为 base = 空快照 S0：彻底白手起家，首次记录把全部现有文件记为整体变更（+N 文件） |
| O5 | `cmp -r` 移除 Compose | 成员 opt 沉淀自由池（对象不毁）；仅 `opt -r` 毁对象 | 连同成员一起销毁（更危险） |
| O6 | 链接同步冲突时 | 阻塞：留下步骤，本次命令不执行（退出码 1） | 仅告警继续执行命令 |
| O8 | 短 id 规则 | ≥4 位十六进制唯一前缀 | 8 位起 |
| O9 | 编辑器选择链 | `$POC_EDITOR` → `$VISUAL` → `$EDITOR` → `vi` | 仅 `$EDITOR` |
| O10 | 冲突交换文件格式 | 每冲突文件一份，标准冲突标记文本 | 自定义结构化格式 |
| O11 | `proj <path>` 目录不存在 | 自动创建（mkdir -p） | 报错拒绝 |
| O12 | 初始 Compose 名 | `proj` 初始化时自动建 `main`；`cmp -r` 移除最后一个 Compose 时拒绝 | 首名取项目名；允许零 Compose 状态 |
| O14 | 跨项目 opt 对应关系 | syncmap 显式映射表（同步时登记） | msg+time 启发式匹配 |
| O15 | 同步收敛序 | 公共前缀 + 本地独有 + 目标独有（确定性） | 以时间戳重排 |
| O16 | `--dry-run` 范围 | 全局，作用于一切改写命令；输出将新建/改写的 opt（真实 id）、涉及表、冲突预检；不建步骤、不触发链接同步 | 仅限 opt / cmp |
| O17 | v0.3 新增旗标/形态的去留 | `opt --amend`、`show <opt-id>`、裸 `lk` 列链接、`--yes`、`--no-sync`、`--no-pager/-p` 全局化、`--quiet/--verbose` 全部进 1.0 | 逐个取舍 |
| O18 | `diff` 命令 | 新增（§5.6）：0/1/2 个 id + `--stat`；与 `show <opt-id>` 有意重叠 | 不加，靠 status 统计 |
| O19 | `--yes` 与步骤协议的关系 | 危险命令带 `-y` 立即执行；不带 `-y` 且 `-s` 则留待办步骤 | 危险命令一律建步骤，`-y` 无效 |
| O20 | 1.x 延后实用项 | `--json` 机器可读输出、`opt -e` 改消息、`cmp --rename`、快照文件清单浏览、`opt --split` 拆分 | —— |

沿袭 v0.1 且未被新设计推翻的既定项：哈希 BLAKE3-256（可换）；无暂存区；不检测重命名；空目录不追踪；字节原样存储不改换行符；op 身份含 author/msg/time；`thiserror` 库层错误类型。

---

## 10. 1.0 之后（展望，非承诺）

**cpoc 与链接协作**（v0.5 自 1.0 推迟至此）：`lk` 命令、建档 / `--once` / 自动同步、compose 映射与 syncmap（§1.5、§4.5 为设计输入，links/syncmap 两张表随之引入）、lk 的 targetURI 扩展与链接冲突的自动策略；对象分块与包文件（大库性能）；自定义结构化冲突格式；行级交换的精细判据（行区间不相交即合法）；代数驱动的自动重排（利用交换子群）；全局项目簿。
