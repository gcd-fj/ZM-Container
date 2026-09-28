# 2026-09-28 性能检查与优化

本轮检查覆盖宿主调度、输入、共享 GPU 渲染、静态资源请求与缓存、Ruffle 数据解析、日志诊断，以及启动取消和凭据存储的线程边界。没有加入补帧或修改游戏目标帧率。性能证据区分代码机制、合成测量和真实游戏；自动测试不替代造四/造五实机验收。

## 根据用户诊断确定的重点

用户提供的造五会话约 86.985 秒。第 5.662 秒所在更新中，本地任务关联 `assets/data/cg1_20260921_1.swf`，执行约 1510ms；第 15.279 秒所在更新中，播放器 tick 约 1155ms；输入阶段另有约 97ms 峰值。这些是主线程墙钟时间，不能直接归因于下载、解压或 GPU。

只读检查本地缓存发现，`cg1` 虽然扩展名为 `.swf`，内容实际是 AMF3 对象及 ByteArray 序列，没有 FWS/CWS/ZWS 头。外层解码成功读取 398 个值，其中 397 个为 ByteArray。这个检查不执行游戏对内部数据的处理，不能据此解释或消除整段 1510ms 停顿。因此没有按扩展名提前解压资源，也没有修改官方数据。

## 已落地的改动

| 范围 | 原来的开销或问题 | 改动与边界 |
| --- | --- | --- |
| 静态资源 | 资源 Future 在 AVM 的本地执行器上轮询，HTTP 响应体解压和复制可能占用 UI 线程 | 将 Send 资源工作放到已有 Tokio 工作线程。回到本地执行器后才交付结果、执行游戏回调；退出时取消后台等待，已有原子写入及锁的规则保留 |
| JSON | 遍历每一层对象和数组时都 clone 子树，嵌套越深，重复复制越多 | 消费已解析的 JSON 树，直接移动子节点。保留对象遍历顺序、数字精度、reviver 调用顺序及结果 |
| GPU 资源 | 每次提交 Ruffle 画面都重新创建同一纹理的 TextureView | 随纹理创建一次视图并复用；尺寸改变时一并重建 |
| AVM 日志 | `removeTime` 等连续重复 trace 占满近期历史，并逐条写日志 | 相同且连续的脱敏 trace 合并为一条并保留准确出现次数；文件在第 1、2、4、8…次采样输出。警告不合并，初始化通知和兼容性计数仍逐条处理 |
| 资源诊断 | `.swf` 后缀的配置数据也计入动态影片数 | `dynamic_swf_ready` 改按内容的 FWS/CWS/ZWS 签名统计；该计数不是完整 SWF 格式校验 |
| 慢阶段定位 | 只能看到笼统的 tick 或加载任务峰值 | 增加 JSON 解析/对象创建、AMF0/AMF3 解码/对象创建、ByteArray 解压阶段计时；仍由 `zm_perf=info` 开启，只记录超过 25ms 的调用 |

后台化减少 UI 线程承担的资源工作，不意味着 HTTP 本身更快，也不把游戏脚本移到后台。优化后的 `task_poll` 与旧版本统计范围发生变化，需要结合进程 CPU 采样，不能仅以主线程耗时下降声称总 CPU 消耗下降。

## 审查后保留的设计

- 游戏仍按自身 Stage 帧率推进，保留已修正的截止时间算法。日志中的 124.63Hz 宿主更新不足以证明忙等待；不通过丢输入、降低计时精度或强制帧率来换取表面指标。
- 缓存继续按游戏/版本/桥接隔离，静态资源请求仍合并，鉴权 POST 不重试或缓存。零命中可能是版本更新后的冷缓存，不据此放宽缓存有效性。
- 启动会话编号和取消边界、SharedObject 隔离、凭据操作顺序保持原有机制。密钥环和资源落盘已有后台处理，没有证据支持在这轮更换协议或存储实现。
- GPU 提交耗时不能代替 GPU 执行时间；没有在缺少测量时修改画质、滤镜或位图缓存语义。

## 自动验证与离线测量

回归覆盖后台线程执行、取消与错误回传；重复日志的精确计数、警告保留、脱敏及兼容性计数；真实内容签名；嵌套 JSON、Unicode、深层数组/对象、reviver 删除和回调顺序。JSON 的 AS 源文件与 SWF 测试产物均使用固定版本 ASC/playerglobal 重新生成。

JSON 合成基准位于 Ruffle 的 `profile_json_deserialization` 测试：同一份嵌套树，七轮交替执行原 clone 路径与新路径，分别报告中位数。输入准备、播放器构建不计入转换计时；不代表游戏战斗 FPS 或整个加载过程的提速比例。

本机 release 结果为 **16.167ms → 0.597ms**，新/旧耗时比 **0.037**。该输入刻意包含深层嵌套，用于验证重复复制的放大效应，不代表游戏中的平均 JSON 负载。Ruffle 的 117 项 release 单元测试（包含此基准）通过；工作区 118 项测试通过，1 项既有合成微基准默认忽略。记录位于 `target/perf/optimization-core-release-tests.txt` 和 `target/perf/optimization-tests.txt`。严格 Clippy、格式与差异检查通过，性能采样工具的 9 项测试通过；分别记录于 `target/perf/optimization-clippy.txt` 和 `target/perf/optimization-perf-tools.txt`。

Linux 优化版构建通过，产物为 `target/release/zm-linux`，构建记录为 `target/perf/optimization-release.txt`。本轮未重新打包 AppImage，也未执行 Windows 构建；旧安装包不会自动包含这些改动。

```bash
cargo test --release --locked -p ruffle_core profile_json_deserialization -- --ignored --nocapture
```

## 实机验收

启动优化版并打开细分阶段日志：

```bash
RUST_LOG='warn,zm_player=info,zm_perf=info' ./target/release/zm-linux
```

用相同资源版本分别测冷/热启动，在同一账号、场景与窗口条件下测试面板和战斗。保留卡顿时的完整文件日志与同一会话诊断。新增阶段包括 `json_decode`、`json_objects`、`amf0_decode`、`amf0_objects`、`amf3_decode`、`amf3_objects`、`bytearray_decompress`；阶段可能嵌套，耗时不能相加。

这轮没有自动操作真实账号或产生联网游戏行为。两次秒级停顿的最终根因、优化后的战斗表现和 Windows 实机结果仍待复测；不宣称这些问题已经全部解决。
