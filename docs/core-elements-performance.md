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
|uniform_list|默认Auto已不做布局阶段冷测量；预paint测量/可见范围/空列表|⏳ 执行路径已读，待量化|
|list|可见布局、sum_tree共享、focus查询与离屏保活|⏳ 已定位，尚未完成执行路径核对|
|animation|steady/done状态、时间评估、帧调度、wrapper分配|⏳ 尚待执行路径核对|
|animated_list|shared状态读写、painted map与enter/leave快路径、行构造|⏳ 已读主要wrapper路径，待继续|
|animated_uniform_list|可见帧动画scratch、advance追踪范围、稳定列表|⏳ 已读主要wrapper路径，待继续|

## 量化与修复账本

|ID|反馈与边界|状态/证据|
|---|---|---|
|C01|4096个普通keyed状态执行真实compute_style_internal；没有click/active交互时不应生成4096个clicked Rc|✅ 实际red→green：4096→0；真实prepaint保持active=Some(false)；核心454 passed / 4 ignored；独立本地提交见Git日志|
|C02|真实paint_mouse_listeners注册鼠标click，但元素未聚焦；保留pending mouse/clicked，不创建无用pending keyboard|✅ 有效red→green：字段分配(true,true,true)→(true,true,false)；核心455 passed / 4 ignored；独立本地提交见Git日志|

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
