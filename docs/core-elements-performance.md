# 核心组件性能与内存优化账本

## 当前任务（2026-11-22）

用户要求高强度优化 `crates/gpui/src/elements` 全部组件，特别是 div；暂不查看或修改独立 `crates/gpui_elements`。本任务是新授权，不恢复旧38项引擎计划的其他批次。

- 基线：`f057c297a4`；工作分支：`perf/core-elements-hotpaths`。
- 分支保留 G09 与前四阶段核验成果，未切旧 main。
- 保护用户 `.idea/Hgpui.iml`；不改 IshiTone 用户工作，不 push/合并/删分支。
- 单 agent 直接执行；每个独立修复先建立确定性反馈、记录旧结果，再修复/功能回归/本地提交。
- 不靠删功能、改变公开调用接口、破坏实例隔离、裁剪/输入/可访问性/动画语义或提前复用 GPU 资源换数字。
- 数字区分 inline struct、实际缓冲/owner、分配次数、操作计数与计时；debug机制结果不等于Release UI/RSS。
- Cargo在仓库根运行，临时 `CARGO_INCREMENTAL=0`，GPU tests单线程；环境故障及时交给用户，不删cache或绕过工具链。

## 全量覆盖入口

18个Rust文件（包含mod.rs），17个组件实现模块；基线约17474行（含测试）。先按实际热路径建立反馈，若某模块没有确认收益，保留原实现而非凑改动。

|模块|重点/已读取的执行入口|进度|
|---|---|---|
|div|Interactivity构造/布局/样式/事件、属性密度、状态分配、子bounds查询与滚动|⏳ 已建立两项状态分配红测与结构体尺寸探针|
|animated_div|共享Interactivity、每帧滚动child_bounds容器、稳定/运动期间帧请求|⏳ 执行路径已读，待量化|
|canvas|inline StyleRefinement、默认构造、布局/paint保留状态|⏳ 执行路径已读，待量化|
|container_query|默认size_full、延迟创建子树与CSS继承、inline样式|⏳ 执行路径已读，待量化|
|measurer|重复实体update/notify、测量布局与不绘制语义|⏳ 执行路径已读，待量化|
|anchored|子树bounds联合/偏移、fit模式、小容器分配|⏳ 执行路径已读，待量化|
|deferred|子所有权take、延后绘制捕获上下文/顺序|⏳ 执行路径已读，待量化|
|svg|共享Interactivity、style执行、资源与变换、glyph/SVG请求|⏳ 执行路径已读，待量化|
|surface|inline样式、native texture引用、平台边界|⏳ 执行路径已读；Windows source类型为空，实机证据需标边界|
|image_cache|wrapper三阶段scope、pending消费者、retain-all显式语义、owner释放|⏳ 执行路径已读，待量化|
|img|布局/异步加载/订阅、动画帧与解码中间缓冲|⏳ 已定位，尚未完成执行路径核对|
|text|普通/Styled/InteractiveText、runs、transform/truncation、layout共享|⏳ 部分执行路径已读，待继续|
|uniform_list|默认Auto已不做布局阶段冷测量；预paint测量/可见范围/空列表|⏳ Auto冷测量优化保持；显式Infer的代表行测量由布局阶段结果复用至prepaint，见C09|
|list|可见布局、sum_tree共享、focus查询与离屏保活|⏳ 已读可见/overdraw测量、SumTree seek及focus汇总；宽度失效和measure_all为有意语义，暂不改|
|animation|steady/done状态、时间评估、帧调度、wrapper分配|⏳ 尚待执行路径核对|
|animated_list|shared状态读写、painted map与enter/leave快路径、行构造|⏳ 已读状态与退出/painted map；painted map clear保留桶，构造全行是此非虚拟API的语义选择，暂不改|
|animated_uniform_list|可见帧动画scratch、advance追踪范围、稳定列表|⏳ tracked表原先逐帧mem::take重建；C08已用稳态分配计数红绿验证|

## 量化与修复账本

|ID|反馈与边界|状态/证据|
|---|---|---|
|C01|4096个普通keyed状态执行真实compute_style_internal；没有click/active交互时不应生成4096个clicked Rc|✅ 实际red→green：4096→0；真实prepaint保持active=Some(false)；核心454 passed / 4 ignored；独立本地提交见Git日志|
|C02|真实paint_mouse_listeners注册鼠标click，但元素未聚焦；保留pending mouse/clicked，不创建无用pending keyboard|✅ 有效red→green：字段分配(true,true,true)→(true,true,false)；核心455 passed / 4 ignored；独立本地提交见Git日志|
|C03|ARIA稀疏属性不应在所有共享Interactivity中占304B；保留默认读、实例隔离、builder与可访问性动作|✅ red→green：slot304→8B，Interactivity1160→864、Div1264→968；专项6、核心458通过；workspace all-targets通过；独立提交见Git日志|
|C04|18组listener Vec的空header与单监听器过量容量|✅ red→green：空header24→8B；single click capacity4→1、请求64→24B，仍1次分配；Interactivity864→576/Div968→680；核心466及workspace检查通过|
|C05|AnimatedDiv稳定的scroll child_bounds应复用buffer；不能每帧替换Vec|✅ 实际red→green：16个子项连续帧同一pointer/capacity16；核心467 passed / 5 ignored；独立提交见Git日志|
|C06|普通/AnimatedDiv子项16→0后ScrollHandle不能残留旧bounds/峰值buffer|✅ 实际red→green：两类len/capacity16/16→0/0，稳定非空复用仍通过；核心469 passed / 5 ignored；独立提交见Git日志|
|C04b|复测发现多项spill与高密度action成本退步|✅ red→green：4回调3/120→2/88请求；dense action5/1000→3/896，恢复Vec adapter；核心471及workspace通过；优化构建复测已记录|
|C07|Canvas不带样式时不应携带1352B可选refinement；保留Styled与RequestLayoutState接口/绘制|✅ red→green：payload 1384→40B，默认构造0次分配；styled Canvas布局/蓝色背景回归通过；组件专项15 passed / 1 ignored，workspace all-targets通过；独立本地提交见Git日志|
|C08|animated_uniform_list稳定tracked窗口避免每帧重建HashMap buckets；动画/滚动语义不变|✅ red→green：10,000-key harness稳定暖机后100帧从400次/156,400B分配请求→0；四类List定向回归和核心库全测通过；workspace all-targets通过；独立本地提交见Git日志|
|C09|uniform_list显式Infer避免request_layout与prepaint重复测量同一代表行；Auto冷路径不变|✅ 实际render回归red→green：稳态每帧回调3→2（Infer测量1＋可见范围1）；uniform_list、四列表相关回归、核心库全测与workspace all-targets通过；独立本地提交见Git日志|

入口命令：

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib core_component_ --offline -- --nocapture --test-threads=1
```

## C01 — 只读样式解析不创建 clicked Rc

实际修改前：4096个default InteractiveElementState全部在 compute_style_internal 的 get_or_insert_with 中生成clicked Rc；样式并未消费任何点击/active特性。两次实际red，最终回归同一路径计数为0。

- 只读取已存在的clicked state；真实mouse/active handlers继续负责按需创建和更新，未共享可写全局默认Rc。
- 在prepaint中用缺失Rc代表inactive，并显式保持keyed元素公开active=Some(false)，不是偷改为None。
- 原实现注释保留；没有更改公开方法/事件/样式API、GPU资源或平台分支。
- 新增内部路径计数与实际prepaint/element-state检查两项测试，2 passed；核心全量454 passed / 4 ignored。
- 实际收益是4096个无用Rc实例及之后每次只读style解析的RefCell borrow被消除，不冒充RSS或整体UI倍率。

## C02 — 未聚焦鼠标控件不创建键盘 Rc

- 正确seam：canvas prepaint注册hitbox、paint调用真实paint_mouse_listeners，外部用AnyWindowHandle驱动draw，避免typed root已update时再draw的测试夹具重入。
- 实际red记录mouse/clicked/keyboard分配(true,true,true)；修复后(true,true,false)。初始两个夹具错误（阶段与root重入）不当作产品运行故障。
- 原pending_keyboard_down初始化注释保留，将完全相同初始化移入is_focused分支。鼠标/拖拽状态创建、键盘focus generation配对与已存在键盘state寿命不变。
- 专项3 passed；核心455 passed / 4 ignored，既有点击、focus移动/ABA、hover与滚动回归均通过。未修改平台/GPU路径或公开接口。
- 收益：每个从未聚焦的mouse/drag控件不再持有一个未消费的键盘跟踪Rc，也少一次相应克隆。

## C03 — 稀疏 ARIA 不内嵌大 payload

- 尺寸红测实际失败：empty ARIA slot=304B，目标为指针大小8B；默认click/focus可访问性动作在旧实现已单独验证。
- 用LazyAriaProperties(Option<Box<AriaProperties>>)保持原字段式builder/内部写法；只读缺省引用一个不可变static默认值，不用unsafe、全局可变状态或once-lock。第一次写入创建各实例自己的payload。
- 保留完整属性写入逻辑，只跳过不存在的属性payload；click/focus/custom action注册仍在该条件外。
- 默认读取不物化、两个实例不共享可写metadata、字符串/数字属性及无metadata动作均回归通过。原inline字段注释保留，公开样式/交互调用接口不变。
- 实测slot304→8B、Interactivity1160→864、Div1264→968、AnimatedDiv1360→1064、Svg1232→936、Img1280→984、UniformList1232→936。List不使用该Interactivity，尺寸未变。
- 不带ARIA属性的这些节点少296B inline payload；有属性的节点转为owned heap payload，多一个pointer/首次物化分配，不声称每种节点总内存都下降。
- 专项6 passed；核心458 passed / 4 ignored；workspace all-targets check通过，未查看或修改独立组件库源码。无GPU/平台资源改动。

## C04 — 空/单/多监听器按实际需要存储

- 首先验证test-only System分配请求探针：线程局部隔离、scope重置与panic恢复2 passed。计数仅Rust成功分配请求/请求字节，不是live heap或GPU/RSS；生产构建无此模块/allocator。
- 真实红测：空click字段Vec header24B；第一个16B Rc listener触发capacity4、一次64B请求。
- 18组私有监听器字段改为ListenerList(Option<Box<SmallVec<[T;1]>>>)：空字段8B无分配，首项保存在首个存储分配中；多项才spill，原type声明注释保留。公开builder不变。
- take/drain保留顺序与未消费项Drop；click/aux/drop/hover捕获移动small payload，释放storage header，单项clone无需第二个buffer。spill与多监听器路径保持全部回调。
- 实际green：空header8B，single click capacity1、一次24B请求；省40B请求且不增加分配次数。
- Interactivity864→576，Div968→680，AnimatedDiv1064→776，Svg936→648，Img984→696，UniformList936→648。叠加C03，Div基线1264→680，inline payload少584B；不含仍存在的style Box及其他外部分配。
- 支持0/1/many、clone/drain顺序、partial drain恰好Drop、单项take后clone不分配等4个存储测试；专项8、核心466 passed / 4 ignored；workspace all-targets通过。
- 多监听器spill可能比原Vec多一个小storage分配，这是explicit权衡，不宣称所有density/总heap/帧时都无条件改善。未更改平台或GPU资源。

## C05 — AnimatedDiv稳定滚动数据复用

- 在实际AnimatedDiv/ScrollHandle视图驱动连续帧。原实现有效red：16项、capacity16保持，但buffer地址在下一帧不同；RHS新Vec在旧Vec释放前分配，不依赖RSS或allocator地址偶然复用。
- 原with_capacity替换保留为注释，改为clear+reserve：首次/增长仍预留足够空间，稳定帧同一buffer不再分配；与普通Div既有buffer复用策略一致。
- 连续3帧pointer和capacity均不变，长度16；专项1 passed，核心467 passed / 5 ignored。初始夹具缺AppContext trait导入是测试编译错误，不当产品运行故障。
- 只消除稳定帧重复分配/释放；容量保留可能令收缩后的逻辑分配高于旧版逐帧精确重建，不宣称所有peak/idle内存都降低。0子项的已有清理语义单独核对，不混入此提交。
- 没有修改公开调用、动画时间/easing、child bounds顺序或GPU路径。

## C06 — 完全空内容清理旧滚动数据与buffer

- 实际UI从16个子项变为0，普通Div和AnimatedDiv都red：ScrollHandle仍暴露16个旧bounds并持capacity16。
- 0子项分支现在将child_bounds置为Vec::new()；清掉错误的旧item数据，释放无用buffer。稳定空帧没有buffer可重复释放；非空稳定帧保留C05复用。
- 两个UI回归len/capacity均16/16→0/0；C05连续三帧指针复用仍通过；核心469 passed / 5 ignored。
- 没有改变公开接口、边界顺序、动画或GPU路径；只处理明确完全空内容，不每帧对非空buffer shrink。

## C04 时间成本复测（不隐藏权衡）

debug同机median5每节点μs，旧Vec/compact：0项0.026/0.076；1项0.275/0.344；1项带clone0.487/0.575；4项0.577/1.800；16项带clone2.752/3.770。debug额外调用开销与spill确实更高，不能从省内存推出提速。

新增小型独立优化构建harness `tooling/core-element-bench`，直接用path导入生产ListenerList及测试分配计数器；无改写实现，无GPU依赖，opt-level3/7轮交替顺序。优化构建实测每节点ns旧Vec/compact：

|项数/clone|旧/新请求次数与字节|median7 ns旧/新|
|---|---|---|
|0/false|0/0，0/0|7.66 / 1.32|
|1/false|1/64，1/24|97.34 / 88.09|
|1/true|2/80，1/24|175.36 / 92.21|
|4/false|1/64，3/120|102.61 / 304.37|
|16/true|4/704，6/760|489.54 / 760.27|

证明空/单/单clone收益，但多项spill是确认的额外分配/时间成本，继续检查减少spilling增长或对高密度种类保留旧存储；不将上述机制ns视为实际应用帧时。

## C04b — 密集路径回退/补偿而非掩盖回归

- 原C04优化构建确认4回调/16回调比Vec慢；实际新红测分别为3次120B增长和dense action5次1000B（目标2次/3次）。
- 多项第一次spill直接预留Vec惯常capacity4，避免capacity2→4额外realloc；第一项仍capacity1/一次24B。
- action注册通常高密度，恢复该字段原Vec adapter；17种稀疏字段保留空/单项收益。原ListenerList字段注释保留。
- 实际green：4回调2次88B；16个action3次896B，与Vec增长成本相同。相比C04 inline payload多16B：Interactivity592、Div696，仍比最初1160/1264少568B。
- 专项13 passed / 1 ignored、核心471 passed / 5 ignored，workspace all-targets通过。
- 直接导入生产实现的优化harness复测median7 ns/节点：0项7.38/1.29；1项99.24/83.28；1项clone170.68/84.24；4项120.57/206.17；16项clone535.14/663.18（旧/新）。
- 4项相对C04原304.37下降到206.17，但非action多项仍有一个额外storage分配/时间成本，明确保留权衡；不宣称所有输入密度都提速。

```sh
# 独立小harness，无GPU依赖；同进程控制，使用生产ListenerList源码。
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=target/core-element-bench cargo build --manifest-path tooling/core-element-bench/Cargo.toml --release --offline
target/core-element-bench/release/core-element-bench.exe
```

## C07 — Canvas 样式按需物化

- 修改前真实红测：`Canvas<()>` inline payload=1384B，默认构造分配请求0次；compact目标≤64B，测试因体积断言失败。
- Canvas改为 `Option<Box<StyleRefinement>>`：无样式默认值为 `None`；首次调用既有 `Styled` setter 时才创建 refinement。`request_layout` 仅在存在 refinement 时调用 `refine`，默认 `Style` 及布局/绘制接口保持不变。
- 实际green：inline payload 1384→40B；未样式构造仍为0次、0B分配请求。styled Canvas实际设置24×16尺寸和蓝色背景，布局与场景绘制回归通过。
- `core_component_` 专项15 passed / 1 ignored；`cargo check --workspace --all-targets --offline`通过。只有开启样式的 Canvas 额外在首次样式访问时拥有 heap refinement；不声称全场景总内存必然下降。
- 未修改公开API、平台/GPU路径或独立组件库。

## C08 — animated_uniform_list tracked-map 稳态复用

- 在 `ListState::advance` 中，原 `mem::take(&mut self.tracked)` 将旧 HashMap 带容量移出；本帧将 tracked keys 插入默认空表，下一帧旧表销毁，暖机后仍重复申请桶。
- 修改前 red：固定 Harness/key 集合预热两帧，再计量100帧；稳定窗口出现400次、156,400B allocation/reallocation requests（约4次/帧）。计量仅覆盖每帧 `advance`，计数器为测试线程局部；不含 Harness 初始化。
- 增加 `previous_tracked` 作为双缓冲，与 `tracked` 每帧交换角色并清空待写表。读前一帧索引与构造当前帧索引仍分属两表；key查找、可见跟踪区间、动画判定及绘制位置逻辑不变。两张表的桶容量都保留，稳定帧不再反复申请。
- 修改后 green：同一100帧反馈为0次、0B请求。代价是稳定窗口缓存两张 HashMap 的有界桶容量（当前帧与上一帧各一份）；这不是总 live-heap 减少，改进点是消除持续分配/释放。
- `animated_uniform_list` 19项回归通过，覆盖插入/删除滑动、anchor、scroll snap、enter/leave、reduce motion和新分配回归；四列表中的变高list、uniform_list、animated_list、animated_uniform_list专项回归均通过；hgpui全库474 passed / 5 ignored，workspace all-targets检查通过。GPU/平台资源未涉及。
- 其他三类本轮审阅结论：普通 `uniform_list` Auto已经避免布局阶段冷测量；变高 `list` 的可见测量/SumTree/宽度失效/focus语义耦合，不能猜缓存；`animated_list` 每行构建是其非虚拟语义，painted map清空时已保留容量。暂不为无量化收益扩大改动。

## C09 — uniform_list Infer 复用代表行测量

- 显式 `ListSizingBehavior::Infer` 的 `request_layout` 已调用 `measure_item(None, ...)` 得到行尺寸用于建立 measured layout；原 `prepaint` 又以相同的无宽度约束调用一次 `measure_item(None, ...)`，其结果作为 content height/item height，重复构造并布局同一个代表行子树。
- 实际 UI 回归使用固定 viewport/key 与可计数的真实 row-builder。修改前，单次稳态重绘的 builder 调用计数为3：Infer request_layout测量、prepaint重复测量、一次可见行构建；测试以目标2次运行red。
- `UniformListFrameState` 暂存 Infer 已测得的 `Size<Pixels>`，prepaint复用该结果。Auto 仍在 prepaint 执行唯一必要测量，不在 request_layout 冷测量；可见范围构建、布局约束和绘制不变。
- 修改后相同实际重绘为2次：代表行测量一次＋可见范围构建一次。该反馈直接证明少一次完整 throw-away row tree构建和布局，不外推为整体帧时倍率。对有副作用的 row-builder，回调次数变少属于性能优化的预期；builder 仍应可重复且遵循元素构建约定。
- uniform_list回归和list/动画列表相关回归通过；hgpui全库与workspace all-targets检查通过。未涉及平台/GPU资源。

## 基线结构体尺寸（Windows debug测试配置）

尺寸是inline payload，不含Box/Rc另行分配的堆数据，也不是全应用内存：

|类型|字节|
|---|---:|
|Div / Interactivity / AriaProperties|1264 / 1160 / 304|
|StyleRefinement / Style|1352 / 1328|
|AnimatedDiv / Canvas<()>|1360 / 1384|
|Text / StyledText / InteractiveText|64 / 104 / 200|
|Svg / Img|1232 / 1280|
|List / UniformList|1384 / 1232|
|Anchored / ContainerQuery / Measurer|104 / 1368 / 64|
|Surface|1360（Windows无可构造source；不是Windows运行占用）|

这些尺寸用于下一项反馈门，而非凭数字直接重构。后续每项追加原反馈、定位、方案理由、实际红绿/指标、平台限制及独立提交。调查中的候选不标为已修；尺寸探针不是完整heap/RSS测量。
