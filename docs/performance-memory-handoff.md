# Hgpui 性能、内存与 Grid 工作交接

> **当前状态以本文件为交接入口。** 用户于2026-11-26要求核心组件优化到此为止：本轮仅继续并收尾已开始的 `crates/gpui/src/elements` 工作，停止后不自动再优化其余组件，也不查看/修改独立 `crates/gpui_elements`。C01–C10及逐项证据见[核心组件账本](core-elements-performance.md)；四类 List 已审阅，三个有量化收益的修复独立提交，变高 `list` 保留原实现。任何后续组件优化须等用户新指示。G09及前四阶段本机核验已完成，旧38项其他引擎批次不自动恢复；Metal/T14实机及整应用Release A/B限制保留。

## 1. 接班开工步骤

1. 到仓库 `D:/WswDay2022/projects/study/Hgpui`，先执行 `git status --short --branch`、`git log -8 --oneline`、`git branch -vv`。完成标准：确认实际分支与 HEAD，区分用户改动和本轮工作；不要依据文档缓存自动切 main。
2. 阅读本文件的状态表与下一步。只有调查具体问题时，再读原始审计的该 ID 详述；已完成项按需读对应修复记录，不重新全仓审计。
3. 继续优化时先建立明确反馈：最小失败回归、操作计数或可重复基准；记录修改前结果，再实施。完成标准：相同路径修改后转绿，功能边界和相关后端覆盖。
4. 每个明确修复独立本地提交，报告已完成/待验证/未开始。push、合 main、删除分支仅按用户新的明确授权执行；前面一次发布清理已经完成，不能视为所有未来分支自动发布授权。
5. GPU 资源改动同轮接通 DX/WGPU/Metal；无 Mac 时完成代码、静态核对及测试入口，并写明未实机验证。当前用户没有实体 Mac，决定先做代码层面检查，不安装黑苹果或工具链绕过。
6. 核心组件优化轮次已按用户2026-11-26指示停止；仅查阅 `core-elements-performance.md` 回顾C01–C10、验证范围与未完成模块。不要自动续做其余核心组件或独立 `hgpui_elements`；等待用户新任务。G09/前四阶段不重做，旧38项剩余引擎路线不因本轮自动启动。

## 2. 版本、分支与保护对象

### 核心组件轮次（2026-11-22 至 2026-11-26；已停止）

- 基线 `f057c297a4`，工作分支 `perf/core-elements-hotpaths`，从已有 G09/核验成果创建，未合旧 main。最后一个优化提交为 `5518043d00`，收尾文档提交为 `835fe6502b`；之后 `4796b71dbb` 增加了 `Deferred::ignore_mask()`。当前 HEAD 以 Git 为准。
- C01–C10记录了该轮实际完成的修复；其中Canvas、div交互/状态、AnimatedDiv滚动缓冲、animated_uniform_list跟踪表、uniform_list Infer测量、animated_list行ID等分别提交。11 月 26 日收尾时，四类List专项67 passed / 1 ignored，hgpui全库476 passed / 5 ignored，workspace all-targets检查通过；11 月 28 日的数据见[单独验收记录](acceptance-2026-11-24-to-28.md)。
- 这不代表17个模块全部完成或每个都已优化：账本仍标未量化/未完成的模块仅作为历史状态，不因本次闭环自动继续。变高 `list` 未做猜测性重构；独立 `crates/gpui_elements` 未查看/修改。
- 用户已要求组件优化到此为止；后续优化待新指示。保留用户 `.idea/Hgpui.iml`；实现均为独立本地提交，不发布或自动合并。

### 已完成的引擎轮次与停止门（2026-11-22）

- 已用 Git 确认 main 在 `bcdceb7b4e` 合入第 3 组及交接文档；相对本地 origin/main 跟踪引用领先 6 个提交，未重新查询服务器。
- G09 工作分支：`fix/shadow-cache-cooldown`，从上述 main 创建；不自动发布或合并。
- 用户要求 G09 后停止其他 ID、先核验前四阶段；本机核验已完成，最终记录为 `first-four-stages-verification.md`。旧引擎路线保持停止；后续核心组件轮次也已收尾，Mac/整应用 A/B 限制保留。

### 整理前历史快照（不代表当前分支；先用 Git 复查）

- 当前分支：`fix/core-quadratic-hotpaths`；整理前 HEAD：`4ab93d3ce5`。
- 本地 `origin/fix/core-quadratic-hotpaths` 跟踪引用也是 `4ab93d3ce5`；说明本地记录已有远端跟踪。2026-11-22 实时 `ls-remote` 返回 RPC/curl 52 Empty reply，未重新证明服务器状态，也没有重试/推送。
- `main` 与本地 `origin/main` 为 `d7aed1aa7b`，包含 Grid 与审计第 1、2 组；**第 3 组尚不在 main**。开始 G09 时保留第 3 组提交作为代码基础，或在用户授权后正确整合，不直接在旧 main 重做 R01/G01/R02。
- `feature/taffy-style-parity`、`fix/perf-ownership-render-wiring` 曾按用户要求推送、no-ff 合入 main，随后本地与远端临时引用删除。保留备份/历史分支，未授权批量推送/删除它们。
- 保护用户已有 `.idea/Hgpui.iml` 改动：第1–4阶段的实现未纳入该文件；之后的 `b472ed6ded` amend 已提交一行测试源目录配置，11 月 28 日验收时该文件无未提交改动。IshiTone 工作不覆盖、不回退、不代为提交。
- 本次交接文档提交会让当前 HEAD 超过上面的整理前快照；最终整理提交以 Git 日志为准，不维护文档对自身提交 hash 的循环引用。

### 环境与已知限制

Windows MSVC，Rust/Cargo 1.98.0 的本轮测量基线；i7-4790 4核8线程、约32GiB RAM、RX580 2048SP + Intel HD4600。具体工具版本与实际 GPU 选择继续工作时查询，WMI AdapterRAM 不当作可用显存预算。

D 盘曾 ENOSPC=0，暂停后用户清理至约123GB；2026-11-22 查询仍约82GB可用。构建产物增长快，先检查空间。占用、磁盘/权限、无法结束进程等环境问题及时交给用户，不自行反复绕过或删除 cache。

本机增量编译有历史 ICE，因此记录命令使用临时 `CARGO_INCREMENTAL=0`，未永久修改环境。GPU tests 单线程运行；WGPU GPU 检查优先独立 `shader_validation` 进程，避免与 Cosmic 测试并行造成驱动堆损坏。

用户实机反馈：Release 软件初始约 **15MB**，对结果满意。这是用户观察，不是标准化 live-heap/RSS分类测量，更不是单项优化因果证据。

## 3. 劳动成果索引（按需读取）

| 文档 | 用途与触发条件 |
|---|---|
| [原始审计基线](performance-memory-audit-baseline-2026-11-15.md) | 查询某个 G/R/T/P ID 的根因、旧位置、严重性与确定性。38项，基线e484；不是当前状态表。 |
| [Grid 完整接口与迁移](taffy-style-parity.md) | 使用/迁移 grid 模板、repeat/track 工厂、定位及 JSON 时读。 |
| [Grid 定位层叠修复](grid-location-refinement-fix.md) | 调整 hover/focus/Auto/Refineable/Range时读，含红绿与兼容边界。 |
| [审计第1、2组修复](perf-ownership-render-wiring-fixes.md) | 文字池所有权、atlas失败事务、Mac callback、DX smoothing问题时读。 |
| [第3组核心优化证据](core-hotpath-optimizations.md) | FocusMap释放、空间树平衡/精确端点、cached-view依赖捕获及夹具数据时读。 |
| [G09缓存回落修复](shadow-cache-cooldown-fix.md) | 无新insert/空闲回落、cached replay保护、三端atlas retirement与真实缓冲/页释放证据。 |
| [前四阶段正确性与收益核验](first-four-stages-verification.md) | 八个ID的源码边界、本机复测、同机机制对照、G09全热成本修正与实机验证限制。 |
| [11 月 24–28 日验收](acceptance-2026-11-24-to-28.md) | 日期范围与 amend 净差异、组件优化与新增 API 的复测、回归强化及验证边界。 |
| [Gaussian pass 工作区优化](render-blur-pass-regions.md) | 三端共享采样依赖、逐像素对照和硬件计时入口。 |
| [渲染管线冗余巡查](render-pipeline-resource-audit.md) | 用户要求对已流畅的 UI 减负；隔离深度、scratch、根目标、几何提交的复现和验证，不是整应用卡顿排查。 |
| [Window state 热路径](window-state-hotpath.md) | `use_state` / `use_keyed_state` 借用 ID 栈命中、跨帧与缓存重放回归、分配及耗时基准。 |
| [图片背景与 Resize 内存](image-resize-memory.md) | 图片/隔离裁剪/背景模糊对照，DX11 驱动资源回落和静态图片解码。 |
| 交叉审查首批正确性修复 | [Infer 行高恢复按阶段测量](core-elements-performance.md#c09--uniform_list-infer-复用代表行测量历史已撤销)；[隔离组边界与 paint layer](render-pipeline-resource-audit.md#交叉审查修正--隔离组边界优先于空间合批)。 |
| [交叉审查功能一致性后续](optimization-correctness-followup.md) | WGPU 变换后剔除、Deferred 祖先裁剪、缓存与 AccessKit 动作；未将正确性回退解释为提速。 |

原始完整报告最初在 `D:/WswDay2022/tmp/Hgpui_Performance_Memory_Audit_2026-11-15_e484.md`。仓库中的基线副本在本次整理前已存在且未跟踪，SHA-256与原件一致：`c9e46f5833417bfeb0a373af8fa69388f413f94a0cb3430de9cd2de3dedb77ae`。纳入版本管理时保留原文，不把“报告在仓库外/未修”等历史描述改为当前结论。原行号按基线查，当前源码重新定位。

## 4. 已完成与验证边界

### Grid 与使用接口

| 工作 | 状态 | 提交 |
|---|---|---|
| 完整 Taffy Grid 轨道、重复、隐式/命名/对齐与共享属性透传 | ✅ 代码与本机回归完成 | `b328fa822c` |
| 展示示例及迁移文档 | ✅ 双后端编译完成 | `27bf6b112d` |
| 定位按四端点递归细化、Range/optional补丁与旧JSON读取 | ✅ 红绿及焦点场景通过 | `df094c1c83` |
| GPUI 风格 `track()` / `repeat()`、`grid_template_cols` 短名 | ✅ 13项外部调用及文档示例通过 | `e484b5d6d4` |

不再让用户拼枚举作为常用入口：

```rust
use hgpui::{div, px, repeat, track, Styled};
let content = div().grid()
    .grid_template_rows(repeat(3, [track().size(px(40.))]))
    .grid_template_cols([track().size(px(120.)), track().fr(1.)]);
let flexible = track().minmax(px(120.), track().fr(1.));
```

保留旧结构/JSON和便捷方法的布局含义；`fr`是auto最小值，`flex`是0最小值。TaffyTree 的 calc resolver 本身返回0，不伪装可用calc；subgrid/masonry/writing-mode等未提供算法不属于透传欠账。用户先前看到IDE E0599，实际 IshiTone cargo 识别方法、只报 E0061 缺模板参数；没有擅改其调用代码。

### 审计第1、2、3组

| ID | 当前状态 | 修复提交 | 证据/限制 |
|---|---|---|---|
| T01 | ✅ 本机修复验证 | `3d4a82bbf7` | 3个Arc生命周期红测转绿；池中parent引用清除、活跃lease保持；缓存复用保留。 |
| G03 | ✅ 三端代码、本机验证；⏸ Mac实机 | `b5c93d241f` | guard原需求、局部创建+首分配成功才发布slot；Core/DX/WGPU回归通过。修改前red未执行，不谎称已见运行旧故障。 |
| T14 | ⏸ 实现/静态核对完成，Mac编译运行待验证 | `9ad6e0ab60` | ObjC output dealloc唯一回收Rust Box，每帧借用、retain防重入关闭；3个Mac回归未在本机执行。 |
| G04 | ✅ 本机正确性验证 | `2c7a8d78b9` | 两个WARP红绿：黑像素、增长后SRV越界；shared实例buffer只切shader，quad/shadow/poly偏移正确。 |
| R01 | ✅ 本机回归/夹具验证 | `4c8d5da9b1` | final Drop队列；无释放不扫描、不锁table；窗口更新锁外；5功能回归。 |
| G01 | ✅ 本机结构/顺序验证 | `423f917d53` | 12路等深split6/7；精确max_corner防float少包围；7回归含随机参考和细条反例。 |
| R02 | ✅ 本机回归/夹具验证 | `6987fcbb64` | global+top scope收集；nested merge、clear、release、panic恢复；修复共享ID遗漏；7回归。 |
| G09 | ✅ 本机修复闭环；⏸ Metal编译/实机 | `375b5188d5`、`3c663d3a49` | 7项专项；CPU图像/Window atlas 80→64；DX/WGPU原生页3→1；空闲、replay/close及全热不扫描覆盖。 |

记录提交：第1/2组 `e8ec127072`；第3组 `4ab93d3ce5`。前两工作分支合并main的提交为 `d7aed1aa7b`。

### 最后实际验证（第3组完成时，不等于未来HEAD）

- 核心 **444 passed / 2 ignored**；两个ignored是本轮手动基准，已分别运行。
- 组件库 **142 passed**。
- Windows **46 passed / 1 ignored**；ignored是会启动GUI的字符映射表既有测试。
- 渲染契约 **43 passed**；WGPU `shader_validation` **1 passed**。
- 工作区全部targets、Windows wgpu feature编译检查通过。
- 普通重复警告在 element.rs/delegate.rs，是基线警告，不为消警告乱改其他功能。

量化证据分别详见修复记录：R01无释放夹具F4096/P256/20批检查量21053440→0，debug elapsed861.367→1.983ms；G01同2048输入height188→5、path191301→7557；R02固定200个局部读取、prefix4096 elapsed110.891→0.279ms，依赖输出错误0→200。**不将夹具倍率当整体Release UI提速**。

## 5. 保留路线图与已完成的 G09 验收门

用户批准的大组顺序：第1所有权 → 第2DX接线 → 第3二次放大 → 第4预算 → 第5无用GPU分配/overdraw。前三组及第4组的 G09 已完成源码修复；T14/Metal 编译或实机验证仍挂账。11 月 28 日按新要求巡查渲染冗余，第 5 组 G07/G08/G05 已实现并完成本机相关验证，见[专项记录](render-pipeline-resource-audit.md)。

4A/G09 及前四阶段本机核验已完成；第 4 组其余缓存预算（4B–4D）不因此自动恢复。

| 次序 | ID | 打算 | 完成门 |
|---|---|---|---|
| **4A 已修复/已核验** | **G09** | 帧边界回落及有限空闲清理帧；保护热缓存、重放与安全在途使用 | 红绿、真实缓冲/页释放及热路径成本复测完成；当前停止，不进入4B。 |
|4B|G02 + T02|配套GPU glyph/SVG与CPU raster_bounds、wrapper字号组预算/驱逐|固定集合warm稳定；长期字号/DPI churn受预算；不能只清CPU放任GPU占位。 |
|4C|T03 + T04|默认图片cache、消费者/请求/解码预算，CPU eviction连逐窗GPU撤销|跨窗共享/重载/离屏/pending/失败恢复正确；显式retain-all仍可用。 |
|4D|T05|动画图聚合帧/字节和并发预算，之后再评估流式frame窗口|大动画可解释拒绝/降级；小动画行为保持，预算并发峰值有证据。 |
|5A|G07|隔离target按实际最大同时深度或进入深度按需创建|两个顺序group只一页、真正嵌套仍正确，三端验证。 |
|5B|G08|分离scene target与Gaussian scratch，只有实际blur创建ping/pong|纯clip/颜色路径无闲置Gaussian页，blur正确性保留。 |
|5C|G05|Metal fullscreen triangle vertex count统一3|shared shader/draw契约、Mac静态核对及实际验证状态如实记录。 |

### G09 验收闭环（本轮已实施；证据见修复记录）

1. 只读定位 `crates/gpui/src/shadow_cache.rs` 的begin_frame/lookup/insert，以及window.rs的shadow使用/atlas.remove接线；不用重新打开完整38项报告。
2. 用现有crate测试模拟稳定阴影逐帧完成烘焙。所有活跃key不能被硬删，因此先产生>64缓存，再退出大多数，仅访问一个已缓存key、完全不制造新miss。完成门：现版确实无法收敛的失败回归和key/图像数量证据，而非RSS“看起来没降”。
3. 在合适帧边界/闲置路径trim过期候选，并把evicted RenderImage交给Window撤销atlas；完成门：保护当前/近期/in-flight读取，不跨后端提前复用texels。
4. 先修已有64软目标的回落，不随意替换成拍脑袋字节硬值。可配置byte预算是进一步设计；过量当前可见内容采用合法fallback，不破坏阴影功能。
5. 验证稳定cache命中、烘焙预算/稳定门、不足安全帧龄保护、异常/close/drop、GPU占位撤销；diff/check与相关tests通过后单独本地提交并报告。

全局预算原则：byte accounting分CPU heap/GPU页/pending tasks；不每帧shrink；保护当前可见和GPU在途；量化字号不能让bounds/raster不同步；不以静默丢功能换内存。不承诺未经测量的具体降幅。

## 6. 38项审计统一状态账本

S/C含义与详细触发/旧位置见原始审计；C1是机制确认，不是已测得所有业务场景性能损失。✅为已实现且本机相关回归验证，⏸明确未开始/待验证，不重新标已修项为当前高危。

|ID|原严重性/确定性|当前状态|后续位置|
|---|---|---|---|
|T01|S1/C1|✅ 修复|第1组记录|
|G03|S1/C1条件|✅ 代码/本机；⏸Metal实机|第1组记录|
|G04|S1/C1|✅ 修复|第2组记录|
|R01|S1/C1|✅ 修复|第3组记录|
|G01|S1/C1|✅ 修复；附带精确裁剪|第3组记录|
|R02|S1/C1|✅ 修复；附带共享依赖正确性|第3组记录|
|G02|S1/C1|⏸ 未开始|4B GPU glyph/SVG预算|
|T03|S1/C1策略|⏸ 未开始|4C 默认图片cache/请求预算|
|T05|S1/C1机制|⏸ 未开始|4D 动画聚合预算|
|T14|S1/C1|⏸ 已实现，Mac编译/运行待证|第1组验证债|
|G11|S1/C1条件|⏸ 未实现，真实失败调度/峰值待测|主计划后失败帧staging；native依赖保留已核验|
|R09|S1/C2|⏸ 未实现，Unix条件待运行|主计划后Unix pipe/executor|
|G07|S2/C1|✅ 按真实深度分配；⏸ Metal 实机|5A，渲染巡查记录|
|G08|S2/C1|✅ Gaussian scratch 按需；⏸ Metal 实机|5B，渲染巡查记录|
|G09|S2/C1|✅ 本机修复/核验；⏸Metal编译/实机|4A及核验记录；当前保持停止|
|T02|S2/C1|⏸ 未开始|4B CPU glyph/wrapper缓存|
|T04|S2/C1|⏸ 未开始|4C remove_asset/GPU image tile|
|T07|S2/C1|⏸ 未开始|主计划后EditableText整文复制|
|T08|S2/C1|⏸ 未开始|主计划后可见行/glyph范围|
|T09|S2/C1|⏸ 未开始|主计划后DecorationRun过预留|
|T10|S2/C1|⏸ 未开始|主计划后List焦点查询|
|T11|S2/C1|⏸ 未开始|主计划后Cosmic冷glyph双raster|
|R04|S2/C1|⏸ 未开始；不是对象泄漏|主计划后arena/容器冷却回收|
|R05|S2/C1|⏸ 未开始|主计划后深ID共享/hash|
|R06|S2/C1|⏸ 未开始|主计划后动画通知合并|
|R07|S2/C1|⏸ 未开始|主计划后keymap输入/context索引|
|G05|S2/C1|✅ 共享提交契约及 WARP 对照；⏸ Metal 实机|5C，渲染巡查记录|
|P01|S2/C1复现|⏸ 未开始|工具：perf别名缺release-fast|
|G06|S2/C2|✅ 局部目标三端接线与 DX11 像素/实卡验收；⏸ Metal 编译/实机、WGPU 实卡计时|[局部路径目标](render-path-compact-targets.md)|
|R03|S2/C2|⏸ 未开始，时序待证|主计划后dead-ID晚通知|
|R08|S2/C2|⏸ 未开始，争用待测|主计划后Entity Drop upgradable-read|
|T06|S2/C2|⏸ 未开始，影响待测|主计划后动画图deadline/可见性|
|T12|S2/C2|⏸ 未开始，Mac RTL序列待证|平台验证后索引映射|
|G10|S2/C3|⏸ **候选可能非瓶颈**|硬件fast-clear验证，避免先改正确性初始化|
|T13|S3/C1机制|⏸ 未开始，热点待测|主计划后SVG parsed-tree cache|
|P02|S3/C1|⏸ 未开始|工具：整数毫秒截断/inf|
|P03|S3/C1文档|⏸ 未开始|工具：delta符号说明|
|R10|S3/C2编译|⏸ 未开始，编译影响待测|工具：Inspector AST往返|

总账：**11 项本机相关修复验证完成＋T14 实现待 Mac 验证＋26 项未修/未实现候选**。G03、G05–G09 的 Metal 编译/实机边界仍保留。4B–4D 的 5 项缓存预算未开始；另 21 项按新任务及测量证据决定，不把候选当作已确认瓶颈。

## 7. 测试与测量入口

本轮命令已跑，但新HEAD必须复测；以下直接复用seam，按需选过滤名，不把cargo编译时间当优化基准。

```sh
# 核心、组件与Windows回归
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_elements --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_windows --lib --offline -- --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p hgpui_render --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_wgpu --test shader_validation --offline -- --test-threads=1
CARGO_INCREMENTAL=0 cargo check --workspace --all-targets --offline --quiet
CARGO_INCREMENTAL=0 cargo check -p hgpui_windows --features wgpu --offline --quiet

# 已存在的专项过滤与手动基准
cargo test -p hgpui --lib focus_cleanup_ --offline
cargo test -p hgpui --lib bounds_tree::tests --offline
cargo test -p hgpui --lib accessed_capture_ --offline
cargo test -p hgpui --lib focus_cleanup_effect_batch_benchmark --offline -- --ignored --nocapture --test-threads=1
cargo test -p hgpui --lib accessed_capture_prefix_benchmark --offline -- --ignored --nocapture --test-threads=1

# 原始Release文本基准；不等于warm UI帧时
cargo bench -p hgpui_wgpu --bench layout_line --locked --offline -- --noplot --sample-size 40 --measurement-time 10 --warm-up-time 2
```

原有 perf-test/perf-compare 引用缺失release-fast，hyperfine当时也不在PATH；不要为跑它们自动安装工具/改profile或将失败当引擎变慢。原始benchmark数据在审计内保存；target/criterion可被清理，不作为唯一证据。

## 8. 接班交付标准

继续某ID后：原反馈转绿、相关功能/资源边界通过、平台接线完整或实机边界明确、每个独立修复有本地提交及证据文档、用户改动未混入。状态更新本文件及相关记录，不改原始审计基线来伪造历史。

发布前重新确认实际origin/main与分支祖先关系；只有用户明确要求时推送/合并/删除工作分支。本次网络查询失败不升级权限或强制重试。报告以当前状态标✅/⏸/⏳，保留尚未验证的条件与可能不成为瓶颈的条目。
