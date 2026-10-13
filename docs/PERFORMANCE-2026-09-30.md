# 2026-09-30 藏宝阁卡顿排查与修复

## 实际日志

00:18:37 重新启动并开启 `zm_perf=info` 的会话中，00:19:26–00:19:40 的 AVM2 帧更新出现 607.601、562.196、523.077、864.692ms 的尖峰。30 FPS 的单帧预算约 33.33ms。00:19:43 的独立窗口中，渲染提交 CPU 墙钟峰值为 5.612ms；它不是 GPU 执行耗时。

网络事件处理峰值 1115.352ms 出现在开场阶段；该阶段非阻塞取出已到达的事件并同步执行回调，不等于网络往返延迟。定时器阶段峰值 327.695ms，内部 ByteArray 解压峰值 167.676ms；两者嵌套，不能相加。GC 的 196.497ms 峰值发生在 cg1 开场加载期间，不能直接解释后续反复发生的帧更新尖峰。

旧日志只有阶段总时间，没有具体方法或时间轴子阶段，尚不能证明某个具体实现缺陷解释了全部实机卡顿。以下修复分别由代码和回归/基准验证。

## 修复

1. 时间轴回退的层级保护：旧实现对每个子对象扫描删除列表，再逐个检查后续兄弟对象是否在删除列表中，最坏复杂度为 O(n³)。改为从上向下一次扫描，记录最近的存活兄弟对象，复杂度为 O(n)（通常哈希查找为常数时间）。保留脚本添加的子对象、原有层级锚点和插入顺序；没有跳过游戏对象或事件。
2. ByteArray 压缩/解压：完成变换后直接交换数据缓冲区，消除原先为最终 ByteArray 重新清零、分配和复制整块输出的过程。保留 zlib/deflate/LZMA、读写位置、字节序、objectEncoding、无效输入及长度限制的行为。解压算法本身和 Flash 同步调用语义不变。
3. 诊断窗口输入：去掉诊断窗口打开时强制屏蔽游戏键盘的条件，保持 egui 控件焦点及切换账号对话框的输入保护。
4. 慢调用定位：继续使用 `zm_perf=info`，增加进入帧、对象构造、帧脚本、退出帧、孤立对象清理等阶段；超过 25ms 的事件/定时器回调输出代码中的方法名，不输出参数、账号、事件内容或网络数据。没有给每个字节码方法都加计时。

## 验证方法

- 现有 TimelineOverlay/TimelineLabels SWF 检查脚本文字、原生标签、翻页与反复回退时的显示顺序。
- 新增混合子对象测试对照原算法，覆盖没有锚点、脚本对象、存活时间轴对象、删除但不重建对象和重建对象。
- 新增 ByteArrayRoundTrip SWF 在阻塞加载与宿主流式加载两种模式下验证三种压缩算法、空输入、跨边界大小、重复使用和失败时缓冲区保留。
- 合成基准交替运行旧/新实现，输入准备不计时；只代表相应代码路径的成本，不代表实际战斗 FPS 或卡顿改善比例。
- CI 的 Ruffle 检查扩展到完整核心单元测试，使新增算法回归在三个目标平台均执行；本机只验证 Linux。

本机验证结果：工作区 **135 项通过、1 项既有基准忽略**；Ruffle 核心 **117 项通过、3 项基准默认忽略**。两项新增基准通过显式命令运行通过。严格 Clippy、Rust 格式、工作流 actionlint 和差异检查通过。

发布模式基准（毫秒，中位数）：

| 场景 | 旧实现 | 新实现 |
| --- | ---: | ---: |
| 时间轴锚点，128 个待替换对象 | 0.223039 | 0.003908 |
| 时间轴锚点，512 个待替换对象 | 11.412945 | 0.016646 |
| 时间轴锚点，1024 个待替换对象 | 85.937730 | 0.049449 |
| zlib 解压并写回 32 MiB 重复字节数据 | 17.405 | 11.397 |

锚点基准使用无存活锚点的最坏搜索情况，五轮交替测量；旧基准包含构造删除列表的成本。解压基准七轮交替测量，从新建的压缩数据缓冲区开始，使用完整旧复制流程与新的实际 uncompress 函数，计时后核对全部字节。它只覆盖所测 zlib 负载；LZMA/deflate 的兼容性由 SWF 回归测试验证，不能把 zlib 提升比例套到它们上面。

对当前缓存中 38 个可直接识别的静态 SWF 做只读检查，最大的单时间轴同时放置对象数为 135；统计不包括脚本动态添加对象、运行时载入影片或未覆盖的资源。因此 1024 对象基准证明的是扩展性缺陷已修复，不能证明本次战斗恰好达到这个对象数量，更不能据此宣称那几次 0.5–0.9 秒停顿已消失。新版的阶段及回调日志用于继续核对真实热点。

原始验证记录在 `target/perf/stutter-workspace-tests.txt`、`stutter-core-tests.txt`、`stutter-clippy.txt`、`stutter-rewind-benchmark.txt`、`stutter-bytearray-benchmark.txt`；这些本地生成记录不随源码提交。

复现基准：

```sh
cargo test -p ruffle_core --release --lib --locked profile_rewind_anchors -- --ignored --nocapture
cargo test -p ruffle_core --release --lib --locked profile_bytearray_uncompress_buffer -- --ignored --nocapture
```

实机复测：结束当前会话后，使用包含上述补丁标记的新程序启动，保持账号、关卡、画质、窗口大小、缓存版本及后台负载尽量一致。

```sh
RUST_LOG='warn,zm_app=info,zm_player=info,zm_perf=info' ./target/release/zm-container
```

诊断中新增标记为 `rewind-linear-v1`、`bytearray-buffer-move-v1`、`callback-phase-trace-v1`。复测应同时比较帧更新尖峰、细分慢回调、任务与渲染提交耗时；不要把宿主 `tick_hz` 当作实际呈现帧率。


## 首轮构建交付（00:39）

Linux release 构建通过，产物为 `target/release/zm-container`，构建记录为 `target/perf/stutter-release-build.txt`；已检查产物为 ELF 且包含本轮三个诊断补丁标记。没有自动重启用户的现有游戏，也没有发布新的安装包。现有进程仍使用旧代码，需退出后启动新程序才会生效。未执行这版的真实账号战斗复测，不能宣称所有实战卡顿已消失。

## 新版实机复测：战斗卡顿仍在（00:40 会话）

用户使用上一轮 release 重启后，00:40:43 开始的新会话出现以下记录：

| 本地时间（UTC+8） | 战斗回调 `BattleMapBase/onEnter()` | 对应进入帧阶段 |
| --- | ---: | ---: |
| 00:42:05 | 556.005ms | 556.629ms |
| 00:42:09 | 1212.653ms | 1213.826ms |
| 00:42:14 | 1098.496ms | 1099.486ms |

这些记录说明主要停顿发生在战斗回调内部。上一轮时间轴与缓冲区优化没有消除它，不能宣称已完成实战卡顿修复。该回调会处理模拟帧并调用物理、动画、技能、角色更新；仅凭外层名字不能确认具体慢函数，也不能据此跳过逻辑帧或修改游戏速度。

另有 `StackUtils/check()` 资源队列定时器及其中约 159–168ms 的同步 ByteArray 解压。它与战斗回调尖峰是不同的观察结果。网络数据回调也有尖峰；仍应区分等待网络与收到数据后的同步脚本处理。

为继续定位，新增 `avm2-stack-sample-v1`：

- 只在开启 `zm_perf=info` 时，针对事件/定时器回调启用内部调用栈观察；沿用解释器每 10000 次函数入口/循环检查的超时检查点，不逐条指令采样。
- 回调持续 100ms 后可产生第一条观察，间隔至少 100ms，每个外层回调最多 8 次；嵌套回调共用限额。每次只保留最内层的 16 个方法名，每个最多 512 个字符。
- 同时测量被跟踪回调中的原生函数，超过 25ms 时输出调用栈，每个外层回调最多 8 条，用于覆盖单个原生函数阻塞、无法到达解释器检查点的情况。首次方法校验超过 25ms 也单独记录。
- 这些是墙钟与检查点观察，不能相加或当成精确的各函数 CPU 占比；开追踪时的原生函数时钟读取有额外开销。关闭追踪后不读时钟或格式化方法名。
- 不记录参数、对象字段、账号、URL、事件内容，也不改动逻辑帧、定时器顺序或超时策略。RAII 清理确保嵌套调用、返回错误或展开栈后不污染下一次回调。

新增 `avm_tracing` 集成测试使用本项目自编的 SWF，在原生 Array.sort 调回脚本期间制造超过 100ms 的工作，检查内部方法名可见、排序与脚本结果保持一致、关闭追踪时无输出，且账号等敏感值的替代哨兵不会进入记录。这是诊断行为测试，不是性能提升基准。限频、嵌套共享预算与异常清理另有核心单元测试。

下一次复现不需要打开诊断窗口；沿用上面的 `RUST_LOG` 启动命令，后台日志会记录内部位置。需要真实战斗复现才能完成剩余热点的修复；当前新增内容本身不声称消除停顿。

本轮诊断回归：工作区 **136 项通过、1 项既有基准忽略**；核心 **119 项通过、3 项基准忽略**；严格 Clippy、格式和差异检查通过。记录位于 `target/perf/stutter-stack-workspace-tests.txt`、`stutter-stack-core-tests.txt`、`stutter-stack-clippy.txt`。没有把临时本地方法校验探针或游戏反编译内容加入仓库。

本轮 Linux release 构建通过，已核对 `target/release/zm-container` 为 ELF 且包含 `avm2-stack-sample-v1` 标记，构建记录为 `target/perf/stutter-stack-release-build.txt`。用户已表示方便再次复现；交付后等待新会话的卡顿调用栈，尚未完成这版实战验证。

## 01:00 实机调用栈与出生点查询补丁

01:01:05、01:01:09、01:01:14（UTC+8）再次出现 568.502、1205.868、1131.604ms 的战斗回调。内部采样反复落在以下链路：

```text
BattleMapBase/onEnter
  GameFrame/Manual -> Juggler/advanceTime -> EntityDeadCmd/execute
    ManCreateManager/onMonsterDie
      onMonsterDieOnBirth -> checkPointAllDead -> getMonsterBirthVo
```

部分采样还在 `MonsterBirthVO/getIsAllDead`、同一路径的 `SimpleTextLog/logErr`。`Function.apply` 的长耗时包含它调用的脚本，不能理解为 apply 本身单独消耗一秒。采样发生于解释器检查点，不能用样本比例推导精确 CPU 占比。

只读检查当前缓存游戏代码发现，`ManCreateManager` 构建出生点时已经使用 `birth.id` 作为 Dictionary 的键，删除也按编号进行；但 `getMonsterBirthVo` 仍逐项遍历并比较编号。死亡回调又在嵌套的出生点、前置条件循环中调用它。

`crates/zm-assets/src/swf_patch/monster_birth.rs` 针对这条实机热点添加快速路径：直接查询已有字典，确认结果对象的 `id` 与参数相同后返回；未命中或编号不一致则继续执行原方法的完整字节码。没有另建需要失效管理的缓存。不存在的出生点继续沿用原查找和后续处理，包括原来的错误日志。

应用前核对完整包名、类、私有字典字段、id 的 int 字段、方法参数和返回类型、构建函数按 id 存储的指令序列，以及查询的每条指令和分支落点。匹配实际游戏编译形式与测试 ASC 形式；不认识的结构跳过补丁，不按函数名字直接替换任意实现。重复应用不会再次插入。快速路径依赖已核对的游戏约定：出生点按自身唯一 id 存入字典。回退覆盖非索引命中及编号改变的情况，不把它当成任意 Dictionary 的通用优化。

缓存补丁版本升级为 **6**，旧主程序不会被误当作已打新补丁的缓存。首次启动会自动重新准备游戏资源；不需要手动清理。应用补丁时，`zm_perf=info` 会输出 `ZM5 guarded birth lookup fast path applied=true/false`。

验证：

- 自编 `MonsterBirthLookup.as` / SWF 在阻塞及流式加载下，对照原方法检查对象身份、空字典、缺失编号、负数、int 边界、删除、替换、编号变动后的回退、嵌套枚举和清空重建。
- 改变查询指令、分支落点、构建函数或目标类时拒绝修改，且输入 ABC 不发生部分变更。
- 对当前本机缓存只读核对，补丁匹配成功；只改动一个查询方法，通过完整 SWF 处理流程与 ABC 编解码往返检查。游戏原文件未改写，反编译内容未加入仓库。
- 工作区回归、三项新增查询回归和严格 Clippy 通过。核心实现本轮没有新增改动，沿用前一轮核心回归结果。

发布模式基准使用 44 条字典记录（40 条普通记录及编号边界用例）、8 轮嵌套检查、每条记录查找 40 个编号。七轮交替运行，中位数：原方法 **15ms**，快速路径 **2ms**。计时在 SWF 的查询循环内，单位为 getTimer 的整数毫秒；每轮校验相同结果。此基准包含正常命中，不含真实战斗中的日志排版、全部死亡处理及资源加载，不能把改善比例换算成整场战斗 FPS，也不能证明 1.2 秒停顿已全部消失。

```sh
cargo test -p zm-assets --release --lib --locked profile_birth_lookup -- --ignored --nocapture
# 可选：对自己本地缓存做只读结构检查，不会执行游戏会话
ZM_PROBE_SWF=/path/to/cached-game.swf cargo test -p zm-assets --lib --locked verify_local_birth_lookup_patch -- --ignored --nocapture
```

检查记录：`target/perf/birth-lookup-workspace-tests.txt`、`birth-lookup-final-tests.txt`、`birth-lookup-local-check.txt`、`birth-lookup-clippy.txt`、`birth-lookup-benchmark.txt`。Linux release 已重新构建，记录为 `birth-lookup-release-build.txt`。剩余实战效果需退出当前进程、启动此产物后确认；资源加载阶段的独立停顿仍应分别分析。


## 2026-10-14：出生点查询优化后仍有战斗停顿

本次使用 `v4760.swf`，日志 `zm-container.2026-10-13.log` 中
`ZM5 guarded birth lookup fast path applied=true`，缓存补丁版本为 6。
10 月 14 日 00:15:51 / 00:15:54（UTC+8）战斗回调分别为 **858.897ms** 和
**577.981ms**，其他死亡处理回调还有约 158–168ms。调用链仍经过
`onMonsterDie -> onMonsterDieOnBirth -> checkPointAllDead`。这说明上一次
直接按编号查询的优化没有消除剩余停顿。

检查该版本的调用链发现：缺失出生点会调用 `SimpleTextLog.logErr`，再调用
`log`。该日志类会给隐藏的 TextField 逐条追加文字并读取滚动位置；Ruffle
每次追加都会重新排版已有全文。现有按单次原生调用 25ms 报告的诊断会漏掉
“许多次短排版在一次战斗回调中累计”的情况。日志中的检查点观察也不能用来
精确分配这部分耗时，不能把上述两次回调的全部时间直接归因于排版。

本轮在游戏资源准备阶段增加有结构校验的 `SimpleTextLog` 补丁：

- `visible=false` 时跳过文本框追加和滚动位置刷新；原来的原始字符串历史、
  错误数组、trace、外部控制台调用和异常处理继续执行。
- 文本框已经显示时保留原来的逐条追加逻辑。键盘打开日志框的原函数已经会
  从原始历史恢复全文；鼠标打开和直接获取隐藏文本框时也补上这一步。
  补齐历史采用游戏原来的键盘展开格式，而不是保留每行额外的 `-->` 前缀。
- 仅修改已核对的游戏日志类，不修改通用 TextField 语义、刷怪判定、死亡事件、
  掉落或模拟速度。结构不匹配、未知跳转、异常范围不合法时跳过；三个方法
  全部准备成功后才写回，重复应用不再修改。
- 缓存补丁版本升为 **7**，游戏自动重新准备补丁；准备时记录
  `ZM5 deferred hidden text log applied=true/false`，无需手动清缓存。

新增 `text-layout-total-v1`：只在 `zm_perf=info` 跟踪的回调内累计 TextField
排版次数、`visible=false` 的次数和墙钟时间；累计达到 25ms 后在回调末尾
输出一条统计。不记录文本或字段值，也不改变排版结果。该统计用于实机复测
核对剩余热点，不能与包含它的外层回调时间相加。

自编 `HiddenTextLog` 合成回归覆盖隐藏日志历史、鼠标展开、可见追加、再次
隐藏/展开、错误数组、trace 顺序、清空后的可见追加以及 try/catch 重定位；
在 Blocking 和 Streaming 两种加载模式下运行原版与补丁版并比较全部 trace。
另检查未知方法结构与重复应用不修改输入。对本机 v4760 缓存只读核对，三个
方法匹配，完整 SWF 管线输出及 ABC 编解码一致；没有执行账号会话或改写缓存。

发布模式五轮交替基准：预先写入 1000 条短日志，然后计时追加 64 条。
旧实现为 **499、504、505、505、527ms**，中位数 **505ms**；修复后五轮均
记录为 **0ms**，表示低于 SWF `getTimer` 的 1ms 精度，绝非没有执行成本。
基准仅计隐藏追加，不含后来主动展开历史的排版。它证明了这条路径能独立
产生半秒级停顿，并不证明实战的全部卡顿都已经消失。

```sh
cargo test -p zm-assets --release --lib --locked profile_hidden_text_log -- --ignored --nocapture
ZM_PROBE_SWF=/path/to/cached-game.swf cargo test -p zm-assets --lib --locked verify_local_hidden_text_log_patch -- --ignored --nocapture
```


本轮验证：工作区 **141 项通过、5 项手工检查/基准忽略**；播放器核心
**120 项通过、3 项基准忽略**。最终重复展开保护有专门回归，相关两项日志
回归、三项累计诊断回归和本机缓存结构检查再次通过；严格 Clippy、源码格式
和差异检查通过。Linux release 构建完成，已核对 ELF 和新增诊断标记。
产物为 `target/release/zm-container`，构建时间 2026-10-14 00:35（UTC+8）。
验证记录位于 `target/perf/hidden-log-*.txt`，不随源码提交。
尚未运行新版真实账号战斗，需用本记录中的 `RUST_LOG` 命令启动该产物后复测。
