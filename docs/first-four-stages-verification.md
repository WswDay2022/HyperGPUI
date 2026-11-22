# 前四阶段正确性与收益核验（2026-11-22）

## 范围与停止门

- 仅核验第1组 T01/G03/T14、第2组 G04、第3组 R01/G01/R02、第4阶段 G09；不实现 G02/T02、图片预算、动画预算或第5组。
- G09 修复已独立提交 `375b5188d5`，核验发现的全热维护冗余修正为 `3c663d3a49`，接班/停止门清理提交 `cf31245156`；其他核验夹具与最终记录另行本地提交。
- 保护用户 `.idea/Hgpui.iml`，不改 IshiTone 未提交工作，不 push/合 main/删除分支。
- 以下时间是同一 Windows/Cargo 1.98.0 的 **debug 机制夹具**；不得换算为 Release UI/FPS 倍率。常驻窗口启动约15MB仅是用户历史观察，不作为逐项因果证据。

## 1. 源码边界核对

|阶段/ID|核对内容|当前结论|
|---|---|---|
|1/T01|活跃 handle 持有 TextSystem；归还 wrapper 前 detach；宽度缓存保留；锁先释放再释放最后 owner|所有权环已断开；活跃 lease 的有效性与缓存复用保留。不能据此声称常驻主窗口启动 RSS 必降。|
|1/G03|原始 tile 需求 guard；try_insert_with 在 GPU资源创建与首 allocation 成功后才发布slot/消费free_list；失败局部RAII|DX/WGPU/Metal 代码路径一致；本机失败事务/正常slot复用有回归；Metal运行挂账。|
|1/T14|唯一 output dealloc 清 ivar 并回收准确 Box<FrameCallback>；逐帧只借用；callback期间retain；正常/注册失败/启动失败均release|静态生命周期核对通过；没有 Mac，无法证明 ObjC/ScreenCaptureKit 编译运行行为。|
|2/G04|三个 smoothing 变体只持 shader stages；各类型共享生产 upload 的实例 buffer/SRV；增长后 range仍使用全帧偏移|WARP生产helper像素与偏移回归是有效seam；新增 GetDesc byte audit，避免只靠字段名字声称省显存。|
|3/R01|final Drop在slots read期间队列发布；take_dropped先释放queue锁再取slots write；无signal只读原子；窗口blur在write锁外|没有锁次序环；弱引用不复活ref_count=0；队列只持ID；已有5项功能回归覆盖。|
|3/G01|overflow13按6/7分裂并向父传播；叶子等深；精确max_corner剪枝；NonNull仅在nodes不变的查找期间使用|结构边界/随机参考/细条与touching语义已有7项回归；不宣称所有空间查询都是log复杂度。|
|3/R02|global+top scope收集；子scope闭合合并父scope；frame clear不清活动scope；release删ID；catch_unwind后pop再恢复panic|机制回归通过；补真实 cached child notify集成，排除“计数正确但Scene仍旧”的假绿。|
|4/G09|帧边界trim；成功AtlasTile身份保护replay；有限空闲帧；retain hot/recent；DX/WGPU顺序上传；Metal同队列marker完成后复用|缓存与atlas回落闭环已通过；核验又定位全热峰值无候选却构建集合的冗余开销，单独修正/复测；不抹掉Mac验证缺口。|

## 2. 已复测的实际机制/资源收益

### R01 — 零释放不扫描

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib focus_cleanup_effect_batch_benchmark --offline -- --ignored --nocapture --test-threads=1
```

20批次结果：

|F handles|P effects|现版visits|现版elapsed ms|
|---:|---:|---:|---:|
|256|64|0|0.480|
|256|256|0|1.781|
|1024|64|0|0.439|
|1024|256|0|1.806|
|4096|64|0|0.504|
|4096|256|0|2.885|

旧机制逐effect与批次尾扫描F，操作数为20×F×(P+1)。4096/256原报告实际记录21053440，本次现版仍为0。旧时间861.367ms仅是历史测量，不冒充本次同机旧二进制A/B；本次强证据是实际扫描计数0及功能边界回归。

### G01 — 结构操作下降且输出保持

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib bounds_tree::tests --offline -- --nocapture --test-threads=1
```

7 passed；本次相同2048个单调不相交矩形：height=5、total_insert_path=7557。原修复前实际红测为188、191301；本轮复测重现修复后结构指标。随机参考 ordering/clear reuse、duplicate/touching、f32细条反例均通过，未用计时阈值代替正确性。

### R02 — 同一进程机制对照与正确输出

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib accessed_capture_same_run_comparison_benchmark --offline -- --ignored --nocapture --test-threads=1
```

从 `6987fcbb64^` 核对旧方法，夹具用同一global set重建旧clone/difference；现版调用真实detect_accessed_entities。每轮200次、每次读一个已在prefix中的entity，预热后5轮交替顺序，取中位数：

|prefix|旧机制median ms|现版median ms|正确依赖输出|
|---:|---:|---:|---|
|256|6.989|0.166|0→200|
|1024|28.627|0.155|0→200|
|4096|126.489|0.168|0→200|

这是同进程捕获机制对照，不是旧整应用二进制或全UI帧时。新集成测试使用真实Entity::cached：初帧父级先读child，后续父级停止该prefix读、child连续命中缓存，再child更新并notify，必须重新绘制蓝色quad；实际专项1 passed，确认notify后的Scene含蓝色quad且先前连续三帧确实命中缓存。

### T01/G03/G04/G09 — 资源与功能收益

- T01：wrapper归还后与最后活跃lease退出后，TextSystem weak无法升级；缓存字宽/复用保持。证明owner可回收，不拿无字节分类的RSS当量化。
- G03：失败资源/首次allocate失败不增加页、不消费free_list；本机核心、DX/WGPU atlas回归已随全量/专项重新通过。原修复前red未执行，保持该历史边界，不改写成已观察旧故障。
- G04：生产helper先前黑中心像素/增长后SRV越界的两项已红绿，本次重新通过。WARP GetDesc 实测quad=19456（64×304）、shadow=832（4×208）、poly=2560（16×160）字节，共22848。历史三份独立smoothing buffer同容量4/64/16已从Git对象核对；现在不存在这三份重复实例buffer，避免22848 ByteWidth请求字节及对应buffer/SRV对象。不是驱动实际驻留显存测量；该组主要收益是正确绘制及消除错误接线，不冒充FPS优化。
- G09：无新insert和最后draw后空闲均CPU image/atlas 80→64，实际像素缓冲983040→786432字节；热点tile不变，close后weak全部释放。DX/WGPU原生1024² BGRA页3→1：引擎持有的页面积字节12582912→4194304，WGPU pending upload同步撤销。不是驱动驻留显存/RSS测量。

## 3. 核验发现的 G09 全热维护成本

首个 G09 完成版本，在全热峰值80项时仍构建两个live set，虽然没有候选，也不会请求cleanup帧。

同进程warm benchmark：2000帧/样本、5轮交替顺序、取中位数；不含bake/GPU/frame pacing。

|热key数|原命中机制 μs/帧|首版带维护 μs/帧|增加|
|---:|---:|---:|---:|
|1|0.322|0.359|0.037|
|64|21.602|23.512|1.910|
|80|29.734|86.839|57.105|

新增 `shadow_cache_hot_peak_skips_scene_scans` 实际red：`no old candidates: do not scan the scene or allocate a live set`。优化只加可证明的早退：没有age>1条目时trim不可能产生victim；全是当前帧hit时needs_cooldown谓词不可能命中。不改变真实cold/cached replay/Metal退休路径。专项7 passed / 1 ignored，benchmark单独执行1 passed。复测80项旧机制28.551μs/帧、带维护33.123、额外4.572（首版额外57.105）；1项0.329→0.383，64项22.370→22.169（差值按噪声解释）。回落资源数量不变。核验发现的冗余开销单独本地提交，最终记录以Git日志为准。

## 4. 全量验证与平台边界

- G09第一修复提交前：核心450 passed / 2 ignored，专项6 passed，DX/WGPU原生retirement各1 passed。
- 最终验证工作区包含两项新增ignored机制测量、真实cached notify集成、全热不扫描回归与DX ByteWidth审计；以下为本次实际结果，不复用旧HEAD数字。
- 新增集成夹具一度编译失败（E0277：RGB Alpha 不能直接 Into<SceneHsla>）；改为与生产绘制相同的 Background::from(rgb).solid 转换。这是测试夹具错误，不当作产品故障或运行红测；后续命令使用显式 && 在首个失败处停止。

|验证入口|实际结果|
|---|---|
|核心全量|**452 passed / 4 ignored**；四项手动benchmark已分别执行，G09维护benchmark修正后再次执行。|
|G09专项|7 passed / 1 ignored（该ignored是维护benchmark，已单独运行）|
|R02真实cached notify集成|1 passed，缓存命中后notify改变实际Scene蓝色quad|
|组件库|**142 passed**|
|Windows原生全量，单线程|**48 passed / 1 ignored**；ignored为已有字符映射表GUI测试|
|渲染契约|**43 passed**|
|独立WGPU shader_validation，单线程|**1 passed**|
|WGPU atlas专项，单线程|**7 passed**；包含G03失败事务与G09退休/空页/待上传释放|
|工作区全部targets编译检查|通过|
|Windows wgpu feature编译检查|通过|
|补丁空白/修改文件Rust语法解析|通过；未全仓格式化|

复用命令（在仓库根，按顺序执行；GPU tests单线程，首失败即停）：

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_elements --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_windows --lib --offline -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p hgpui_render --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_wgpu --test shader_validation --offline -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p hgpui_wgpu --lib wgpu_atlas::tests --offline -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo check --workspace --all-targets --offline --quiet
CARGO_INCREMENTAL=0 cargo check -p hgpui_windows --features wgpu --offline --quiet
```

- 基线element.rs/delegate.rs警告不为消警告顺手更改。
- Mac仅代码/绑定/队列/所有权/API签名/语法静态核对。未安装Mac工具链，未编译、未跑T14及Metal队列/atlas测试，不给“跨平台全部验证通过”结论。

## 5. 实机收益尚未证明的部分

- 未启动/修改IshiTone；它有用户未提交工作，不拿可能不同源码/配置的二进制冒充A/B。
- 未做标准化整应用Release帧时、工作集/live heap/驱动显存A/B。没有实体Mac。
- 已确认各项在明确触发路径的所有权、操作数、输出正确性或资源回收收益；不能据此声称每个常驻窗口都会更快/更省RSS。
- 用户若需要实机闭环：同一Release配置/GPU/窗口大小与内容，比较正常交互、菜单/页面反复打开关闭、产生峰值后回到单一内容并空闲；观察缺阴影/错色/卡顿及稳定后内存。截图/肉眼反馈由用户提供，不自行对抗截图环境。

## 6. 最终状态

✅ 前四阶段的本机正确性、实现边界和明确触发路径的机制/资源收益核验完成；核验发现的G09冗余维护已单独修正并量化复测。其他核验夹具/记录单独本地提交，实际提交hash以Git日志为准，不循环引用本文件自身提交。

⏸ Metal/T14实机编译运行和整应用Release标准化A/B未完成，保留上述限制，不把静态检查或夹具倍率当作完成证明。

当前停止推进所有后续优化，等待用户安排其他事情。
