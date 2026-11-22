# Hgpui 性能与内存审计

## 1. 基线、方法与边界

- 日期：2026-11-15。
- 仓库：`D:/WswDay2022/projects/study/Hgpui`。
- 基线：`e484b5d6d425ea53af397517791984a95df78096`，分支 `feature/taffy-style-parity`。
- 范围：23 个 workspace members、355 个自有 Rust 文件的模块范围。三条审查线覆盖渲染/GPU、运行时/布局/基础工具、文字/图片/组件，按关键调用和资源释放路径审查；不是声称逐函数形式化证明。
- 方法：三条独立只读审查；主线程复核关键所有权环、缓存驱逐、GPU 分配/提交、空间树退化、effect/focus 路径、文本与图片热点；额外核验当前 wgpu 29.0.4 的 staging 契约和实现；运行仓库已有 Release Criterion 文本基准两轮。
- 源代码、配置、锁文件、分支及提交均未改变，没有执行修复或提交。本报告在仓库外；编译及 Criterion 输出仅产生已有 target 下的构建/测量产物。
- Windows MSVC 本机：Rust/Cargo 1.98.0；i7-4790，4 核/8 线程；系统报告 RAM 34,255,020,032 字节（约 31.9 GiB）；RX 580 2048SP + Intel HD 4600。WMI 的 AdapterRAM 不是实际可用显存预算，未把其值作为预算上限。
- macOS/Linux/Web 为源码审查，未进行相应平台实机采样。没有生产 UI 压力回放、GPU capture、heap live-allocation trace 或 P95/P99 帧时测量。
- CPU 活跃 heap、allocator/RSS 高水位、GPU 图集占位、GPU 实际显存页及 staging 分配分开讨论。

### 严重性

- **S0 严重阻塞**：已证实常规场景下持续耗尽资源/崩溃。本次没有确认 S0，不能从输入相关的增长公式直接升级为常规 OOM。
- **S1 高**：真正所有权泄漏、无预算大型历史资源保留、明显二次增长或关键提交/执行阻塞，需优先处理。均列出触发条件，并不表示每个普通窗口都严重。
- **S2 中**：确定或高可信的可避免重复工作、过度预留、容量/调用量放大；应按业务频率量化后优化。
- **S3 低**：局部机制、仅编译时或测量工具解释问题，不优先当作 UI 卡顿根因。

### 确定性

- **C1 确定存在（100% 机制确认）**：所有权关系、显式调用/分配/循环或数学路径无歧义，部分有实际命令证据。不是“已测得固定毫秒或百分比损失”。特定触发条件必须成立。
- **C2 高可信**：关键源码成立，但平台行为、业务调度、实际调用量或锁/输出时序尚需运行核验，可能不成为实际瓶颈。
- **C3 待验证**：操作存在，但硬件等因素可能使其基本无性能问题，不能列成已证实回归。

问题类型另分为真正泄漏、生命周期内保留策略、冗余工作、风险条件、附带正确性错误和测量基础设施。不能统称为“内存泄漏”。

## 2. 排序总表

38 个条目：S1 12 个、S2 22 个、S3 4 个；同一严重性内按确定性和处置价值排序。详述中的平台/输入条件是结论的一部分。

| ID | 严重性 | 确定性 | 发现 | 主要资源/适用条件 |
|---|---|---|---|---|
| T01 | S1 | C1 | TextSystem wrapper_pool 与 LineWrapper 强 Arc 引用环 | CPU/平台文字资源；外部文字系统 owner 释放后 |
| G03 | S1 | C1 条件确定 | 超限 atlas 请求创建零引用页后分配失败，不回滚 | GPU + CPU 扫描；设备能够创建 clamp 后的页 |
| G04 | S1 | C1 | DX smoothing 选择未上传的独立实例 buffer | 附带正确性/帧失败；corner_smoothing > 0 |
| R01 | S1 | C1 | 每个 effect 都全表清理 FocusMap、取得写锁 | CPU/锁；P effects × F focus entries |
| G01 | S1 | C1 | BoundsTree 顺序矩形输入退化为长链 | CPU；未通过 paint_layer 绕开的单调空间输入 |
| R02 | S1 | C1 | cached-view miss 复制全窗口访问前缀 | CPU/临时 heap；大量兄弟 cached views 同时 miss |
| G02 | S1 | C1 | Glyph/SVG atlas 无历史 key/字节预算驱逐 | GPU/元数据；持续新字号/DPI/栅格尺寸 |
| T03 | S1 | C1 策略确认 | 默认全局 asset cache 无图片结果/请求预算 | CPU/任务；不断访问不同图片源 |
| T05 | S1 | C1 机制确认 | 动画图全量解码并保留全部 RGBA 帧 | CPU/加载；大/长 GIF、WebP、多图并发 |
| T14 | S1 | C1 | macOS capture callback 裸 Box 缺回收 | CPU/捕获资源；每次录屏 stream 生命周期 |
| G11 | S1 | C1 条件确定 | WGPU 失败帧不 submit，queue staging 可累计 | staging；持续新上传且没有其他窗口 submit |
| R09 | S1 | C2 | Unix async 环境捕获同步读 pipe，存在背压等待环 | executor/child/fd；慢或大量输出的 shell rc |
| G07 | S2 | C1 | group target 按 start 总数而非最大同时深度分配 | GPU；两个顺序非嵌套 group 多一页 |
| G08 | S2 | C1 | 纯 clip/颜色滤镜也创建 gaussian ping/pong | GPU；完全无 blur 的隔离路径 |
| G09 | S2 | C1 | Shadow cache 超预算后不能自动回落到 64 | CPU + GPU；大量同时活跃阴影后恢复小场景 |
| T02 | S2 | C1 | raster bounds/字号 wrapper 组无历史驱逐 | CPU；连续字号/缩放变化 |
| T04 | S2 | C1 | remove_asset 没撤销旧 ImageId 的 atlas tile | GPU；绘制后反复移除/重载 |
| T07 | S2 | C1 | EditableText warm layout 命中前复制整份文本 | CPU/瞬时 heap；大文本 blink/滚动/重绘 |
| T08 | S2 | C1 | EditableText warm prepaint/paint 扫所有行/长段 glyph | CPU；大文档/超长换行段 |
| T09 | S2 | C1 | 每个逻辑行预留 32 DecorationRun | 活跃 heap；大量单样式文本行 |
| T10 | S2 | C1 | List 焦点不在可见区时扫描所有 focusable 行 | CPU；F 个焦点行、焦点在列表外/远端 |
| T11 | S2 | C1 | Cosmic 冷 glyph 为 bounds/bitmap 各完整 raster 一次 | CPU/瞬时 bitmap；冷字体字形键 |
| R04 | S2 | C1 | App arena/帧容器保留历史峰值，无冷却收缩 | 活跃 heap；大场景后长期小场景 |
| R05 | S2 | C1 | GlobalElementId 展开完整祖先路径并重复 hash | CPU/heap；深 ID 树，Σdepth |
| R06 | S2 | C1 | 同一 view 的动画逐个 Box 下一帧通知，无专用合并 | CPU/分配；M 个动画，不是 M 次 draw |
| R07 | S2 | C1 | action 展示对每候选重扫 keymap，先算 context 再匹配键 | CPU；A 候选 × B 绑定 |
| G05 | S2 | C1 | Metal fullscreen triangle helper 提交 6 vertices | GPU；blur/blit 第二个三角形重复覆盖 |
| P01 | S2 | C1 命令复现 | perf 别名使用不存在的 release-fast profile | 验证基础设施；不是引擎运行时回归 |
| G06 | S2 | C2 | 每个 path batch 全窗口 MSAA clear/resolve | GPU 带宽候选；交错图元造成 B 个 path batches |
| R03 | S2 | C2 | 晚到动画 notify 重新插入已释放 EntityId 的字典键 | CPU 元数据；max_fps timer 晚于子视图释放 |
| R08 | S2 | C2 | AnyEntity Drop 无条件取得全 App upgradable-read | 锁；多线程大量非最终 handle Drop |
| T06 | S2 | C2 | 低 FPS/离屏动画图仍按显示刷新请求下一帧 | CPU/通知；active、非虚拟化、reduce_motion=false |
| T12 | S2 | C2 | macOS 递减 UTF16 indices 重置转换器、反复扫描 | CPU；长 RTL run 的冷 shaping |
| G10 | S2 | C3 | blur scissor 外仍整 target clear，可能有残留全域成本 | GPU 待测；fast-clear/memoryless 可能使其很便宜 |
| T13 | S3 | C1 机制确认 | SVG 每个新尺寸重新读取/parse 源树 | CPU；尺寸/DPI变化，稳定尺寸命中不发生 |
| P02 | S3 | C1 | perf 吞吐率使用整数毫秒，丢精度/小于 1ms 得 inf | 测量数值；固定短迭代/短测量 |
| P03 | S3 | C1 文档契约 | perf delta 的正负含义与注释、Max/Min 改善描述反向 | 测量解释；不是引擎运行时损失 |
| R10 | S3 | C2 | debug/inspector 样式反射生成→AST→重新输出 | 仅编译；默认 release 无 inspector 不发生 |

## 3. S1 高优先级详述

### T01 — TextSystem 的强引用环

证据：`crates/gpui/src/text_system.rs:52-58,309-321,887-899`；`crates/gpui/src/text_system/line_wrapper.rs:17-38`。TextSystem 持有 pool 中的 LineWrapper，wrapper 又强持 Arc<TextSystem>；handle Drop 将 wrapper 放回 pool，没有清池/断环入口。文字系统最后一个外部 Arc 释放后，内部 Arc 仍使其不能 Drop。Text 的 apply_truncation 会借 wrapper，不仅是显式 ellipsis。

这是所有权泄漏，不能靠 TextSystem::drop 内清池补救：环已经使 Drop 不会被调用。成本是每个曾使用 wrapper 的文字系统及其缓存/平台资源，而不是每帧或每关一个窗口新增一份；通常单 App 只创建一套。优先以 Weak 或不强持父系统的 wrapper 设计断环。后续验证应观察最后外部 owner 释放后的 strong_count/析构，而非仅看 RSS。

### G03 — 超限 atlas 分配的孤页

证据：`crates/gpui_windows/src/directx_atlas.rs:129-151,165-169,228-245`；WGPU `wgpu_atlas.rs:160-178,222-248`；Metal `metal_atlas.rs:96-118,171-190`。新页按 clamp 后大小创建/插入，随后 allocator 仍接收原始超限请求，失败不撤销新页。`window.rs:5121-5131` 使用原始 image.size，不因显示范围小而缩小该请求。

DX/Metal 上 16385×1 四色输入可申请 16384×1024 页；若设备创建成功，每次失败留下 live-key=0 的约 64 MiB 名义纹理。失败没有 key，常规 remove(key) 无法找回它；再次申请还会扫描历史孤页，累计尝试可能二次增长。整 atlas/device/窗口析构会释放。必须先校验原始尺寸并对新页首次 allocate 失败回滚；不要把 clamp 当合法性检查。未在本机实际制造这种显存压力。

### G04 — DX smoothing 实例数据未上传

证据：`crates/gpui_windows/src/directx_renderer.rs:887-943,953-985,1811-1837,1881-1886,2225-2245,2326-2342`。普通与 smoothing pipeline 各自 create_buffer；upload_scene_buffers 只更新普通 buffer；draw smoothed 分支选择独立 buffer。不是两个 shader 共用同一资源。

小场景读取未上传数据；全帧索引超过独立初始容量（quad 64、shadow 4、poly 16）时 raw range SRV 可以越界失败。已有局部测试手动更新 smoothed buffer，不代表生产 draw 已接线。这是附带的明确正确性问题，可造成丢绘/整帧 present 前返回，同时浪费已完成上传和前缀 draw；未证明设备重建循环。建议共用全帧实例 buffer，仅切 pipeline state，或准确上传所有独立资源。没有实施修复或 GPU capture。

### R01 — 每个 effect 扫 FocusMap

证据：`crates/gpui/src/app.rs:1681-1685,1762-1781`。flush 循环处理每个 effect 前全表 retain，最后再扫描一次，写锁覆盖扫描。若有 P 个 effect、F 个存活/槽位 focus entries，扫描量约 (P+1)F；释放分支还在写锁内更新所有窗口。

必要的是清理真正释放的 focus，不是每个普通 notify/emit/defer 都在无释放信号时遍历全表。建议最后 FocusHandle Drop 入队/置待清理标志，按批处理并缩短锁内窗口更新。实际锁等待与常规应用 P/F 尚未采样。

### G01 — BoundsTree 长链及二次插入

证据：`crates/gpui/src/bounds_tree.rs:11,264-381`；`scene.rs:217-250`；`window.rs:4605`。MAX_CHILDREN=12；满节点将最佳叶替换为二叉内部节点，没有空间 split/rebalance。单调从左向右、同尺寸、非相交矩形 (2i,0,1,1) 持续沿右侧最佳 subtree 下沉，形成高度 Θ(N/11) 的链，总下降/回传 Θ(N²)。不相交 order=0 的场景不会被最高-order 换位修正成平衡树。

不是一般 R-tree 都有此成本，也不只靠增加 branching factor 能解决。paint_layer 可复用 order 避开逐图元树插入；clear 会释放内容并保留容量，故不是历史对象泄漏。建议真正的 split/rebalance 或有高度约束的 BVH；验证 N/2N/4N 同类输入的插入路径深度与 CPU。

### R02 — cached-view 捕获全局访问前缀

证据：`crates/gpui/src/app.rs:1121-1134`；`view.rs:392-422`。每个 cached miss clone 当时整窗口 accessed_entities，再对结束后的整集合做 difference。C 个兄弟各引入一个新 entity、此前已有 U 项，单 snapshot 复制数为 CU+C(C−1)/2，另有 difference。深层 cached 视图还各存子树依赖，可能 Θ(ND)。

默认普通 view 不走；warm hit 直接 extend 既有依赖，不做 snapshot。依赖保留是必要功能，无关前缀复制不是。建议局部收集器/访问日志、共享嵌套依赖表示，保留失效语义；本次未重写 cache。

### G02 — Glyph/SVG 图集历史键无预算

证据：`crates/gpui/src/text_system.rs:1058-1080`；`window.rs:4886-4905,5040-5055`；三 atlas 的 tiles_by_key/remove。字号/scale 按浮点参数参与 key，SVG 按尺寸生成 key。仓内 glyph/SVG 路径没有帧龄/字节预算 remove；image/shadow 的 remove 不能自动覆盖它们。

K 个历史 key 的元数据 Θ(K)，GPU 页随历史栅格面积累积。每页通常至少 1024²，单色名义 1 MiB、四色 4 MiB；每窗口 atlas 可叠加。固定集合会稳定；整 atlas/window/device teardown 可释放，不是所有纹理永远泄漏。建议保留 in-flight/当前 key 的字节预算与年龄驱逐，尺寸量化必须与实际 raster/bounds 一致。

### T03 — 默认图片 asset cache 无预算

证据：`crates/gpui/src/app.rs:2700-2716`；`window.rs:3906-3921`；`elements/img.rs:539-549,620-661`。全局 loading_assets 永久保存 Shared<Task<Output>>，完成结果仍被共享；只有 remove_asset/App Drop 移除。不同源历史解码结果与 pending 请求同时累积，视图/窗口退出本身不消费回收。

这是 App 生命周期内缓存策略风险，不是强 Arc 环；同源反复 render 命中只加载一次。冷请求有 detached waiter，离屏后仍可继续工作，GPUI 层没有相应解码并发/字节预算。建议默认图片 LRU/预算/消费者计数、取消/并发闸门，同时保留显式 retain-all。

### T05 — 动画图聚合解码无预算

证据：`crates/gpui/src/elements/img.rs:665-724`；`platform.rs:2864-2889`；`assets.rs:43-49,72-75,103-105`。GIF/动画 WebP 解码所有帧再返回，持有各 RGBA/BGRA 帧；显示尺寸/object_fit 没参与解码。自有代码没有聚合帧数/字节限制。

CPU 帧数据至少 4Σ(WiHi)，加编码 bytes、decoder 工作区；例如 1024²×300 完整帧的名义像素就约 1.17 GiB，但未用此输入进行 OOM 测试。第三方单帧限制未被审成“不存在”。这是 eager playback 策略，非每循环重复解码；GPU 仅按实际显示帧上传。建议聚合预算、目标分辨率/流式帧窗口。

### T14 — macOS 屏幕捕获 callback Box 未回收

证据：`crates/gpui_macos/src/screen_capture.rs:102-105,122-125,142-143,173-193,305-313,335-341`。Box::into_raw 存 ivar，帧回调 Box::from_raw 后 forget 保持 owner；但 output class 未注册 dealloc，stream Drop/错误路径只有 Objective-C release，不会自动析构裸 Rust pointer。

每个 session 保留 callback Box及其捕获环境，O(session_count×捕获量)，不是每帧新增泄漏；CMSampleBuffer/图像 CF retain/release 自身有 RAII，不据此称视频帧全部泄漏。建议安全 output dealloc/结束所有权点回收，并覆盖失败路径和异步 stop 时序；本机无 Mac 动态验证。

### G11 — 未 submit 的 staging 保留（条件确认）

Hgpui 证据：`wgpu_renderer.rs:1696-1725,2191-2220,3211-3225`；`wgpu_atlas.rs:262-291`。atlas write_texture 可在获取 frame 前发生，surface 失败直接返回；instance overflow 失败前可已 write_buffer 成功前缀，drop encoder/frame 并不 submit。

新增依赖核验：Cargo.lock 实际 wgpu/wgpu-core **29.0.4**，不是仅按 Cargo.toml 的最低 29.0.3。`wgpu/src/api/queue.rs:124-156,225-228` 明确 native 每次 queue write 分配 staging，下次 submit 才执行，并在 submission 完成后释放。`wgpu-core/src/device/queue.rs:329,379-381,545-566,384-414,1419` 把 staging push 到 pending temp_resources，submit 才 take；不是 dropping Hgpui encoder 自动取消。

因此“持续失败＋每次新上传 P＋无其他共享 queue 成功 submit”时可保留约 O(RP) 的 staging/命令资源。条件机制已由当前依赖源码确认，升级自初步待验证；失败频率、峰值和 Web backend 未实测。另一窗口成功 submit 可排空，恢复提交/Queue Drop 可释放，不是永久所有权环。建议先预计算上传需求、对失败帧的 queue writes 有明确处理；空 submit 是否适合要结合驱动和时序验证。

### R09 — Unix 同步 pipe read 与背压等待

证据：`crates/gpui_zed_util/src/shell_env.rs:181-204`；`util.rs:370-382`。async fn 的 poll 内调用 std read_to_end；stdout/stderr piped，环境 EOF 完成后才调用 process.output().await 消费其他输出。

同步阻塞代码确定；若 rc 写满 stdout/stderr pipe 后无法推进/关闭环境 FD，父进程又先等环境 EOF，就有等待环。第三方 smol/async_process 预先 drain 行为及实际 Unix rc 条件未实测，死锁结论保持 C2，不拿 Windows 推断已复现。建议异步/并发消费全部管道、必要 blocking pool、deadline/child 回收。

## 4. S2 中优先级详述

### G07 — group targets 按总开始次数分配

`directx_renderer.rs:556-580`、`wgpu_renderer.rs:1747-1754`、`metal_renderer.rs:1026-1033` 用 all-start-count.min(2)，但使用按当前 isolation depth。两个顺序、从不重叠生存的 groups 只需 depth=1，却创建第二个 full-window target。4K 一页 BGRA 名义 31.64 MiB；有最大深度 2，并非无界。resize/loss/drop 释放，场景变小无冷却 shrink。建议按匹配边界最大真实深度或进入深度时创建。

### G08 — 无 blur 仍创建 gaussian scratch

`directx_renderer.rs:512-529,158-181`、`wgpu_renderer.rs:1736-1739,1534-1555`、`metal_renderer.rs:1015-1018,576-584`。clip/颜色滤镜也满足 offscreen 条件，而 ensure 同时创建 scene 与两个 half-res ping/pong；颜色/clip 路径不运行 Gaussian。4K 多出的 scratch 名义合计 15.82 MiB/窗口。必要 scene/group/snapshot 不计入浪费；与 G07 不同纹理，不重复计算。建议拆开 ensure scene 与 ensure gaussian。

### G09 — 阴影缓存 64 是软目标，超额后无 trim

`shadow_cache.rs:336-364,367-393`；`window.rs:4337-4348`。当前/上一帧活跃项不可驱逐，故大量持续可见 key 可以超过 64；begin_frame/hit 不 trim，insert 也只尝试删一项，超额 K 后不会主动收敛。峰值→单个已命中阴影可保留 K 条至窗口结束。

CPU 原始像素约 4Σtile面积，GPU atlas 另计；256 个约 200² tile 名义 CPU 约 39 MiB，不是 64×512² 硬上界。面积 cap、稳定门、每帧 bake 预算都存在，非每帧无界新增。建议安全帧龄后的分级字节预算 trim，活跃超额时 analytic fallback。

### T02 — CPU 栅格 bounds 与 wrapper 字号组累计

`text_system.rs:56-58,309-334,875-899`；`line_wrapper.rs:17-38`。raster_bounds 无 LRU，wrapper_pool 按 FontId+Pixels 建组；归还只 reset letter_spacing，不删除字号组/宽度表。每 wrapper 128 ASCII 槽另有非 ASCII Map。持续新字号/DPI/glyph 参数会保留历史键，回到固定字号不删除。固定有限集合会稳定。与 T01 强环、G02 GPU 图集分开；即使断环，长寿命系统仍有此策略风险。建议预算、每组池/年龄限制，避免无截断时无谓借 wrapper。

### T04 — remove_asset 与图片 atlas 撤销脱钩

`app.rs:2683-2685,2778-2786`；`assets.rs:61-68`；`window.rs:5116-5131,5258-5266`。通用 remove_asset 只删 CPU task cache；RenderImage 重载生成新 ImageId，旧 image_id/frame tile 不随 Arc Drop 回收。历史已画图反复移除/重载，会累计旧 GPU 占位。显式 ImageCache remove/clear/release 调 drop_image，窗口/图集销毁也能释放。建议图片专用卸载或 cache eviction callback 接逐窗 drop_image；不能擅自给所有 Asset 假定图片语义。

### T07 — EditableText 命中前整文本复制

`gpui_elements/src/editable_text/element.rs:370-383,691-696`；`gpui_shared_string/gpui_shared_string.rs:116-119`。request_layout 先 SharedString::from(state.as_str()) 才判断后续 version/wrap-width 是否复用旧 size；这是从 borrowed str 创建 owned 文本，不是现有 Arc clone。大 B 字节字段 blink/滚动/外部重绘都会付 O(B) 复制和瞬时额外字符串；不按帧保留历史复制。建议 miss 后物化或 storage 的版本化共享快照。

### T08 — EditableText warm 全行/整段 glyph 遍历

`gpui_elements/src/editable_text/element.rs:837-849` 每次先遍历全部逻辑段再判断可见；`text_system/line.rs:377-380,520-544` 对可见长逻辑段遍历全部 glyph，之后才 mask/raster 筛除。warm prepaint O(L)、长段 paint O(G)，不是 O(可见行/字形)。已有缓存避免重新 shaping，离屏 glyph 不都上传。建议逻辑段 y 索引/二分、wrap-boundary 可见 glyph 范围，保留 IME/选区语义。

### T09 — 单样式行过度预留 decoration

`text_system.rs:543-546,565-573,608-611`；`text_system/line.rs:25-39`。每行 new Vec capacity=32，单样式合并后 len=1 仍整 Vec 进入 WrappedLine/全文 layout cache。L 行冗余约 31L×sizeof(DecorationRun) 且每行分配；未编译测 sizeof，不能填伪字节值。复杂多色行可需要此容量，layout 替换/Drop 会释放。建议小容量、SmallVec 或按实际 runs 预留。

### T10 — List 的外部/离屏焦点全候选扫描

`elements/list.rs:1207-1227`。可见行无焦点后，SumTree 只按 has_focus_handles 过滤，再逐个 contains_focused；焦点在列表外会扫完整 F 个 focusable 行。不是同时构建全部行，只是 CPU 不再受 viewport 上界约束。可见焦点会跳过、找到离屏焦点可早停、无 focus handles 子树可剪枝。建议 FocusId→item 路由/焦点路径定位。

### T11 — Cosmic 冷字形重复 raster

`gpui_wgpu/src/cosmic_text_system.rs:355-360,364-374,395-438`；`window.rs:4897-4903`。bounds 先 render 完整 swash Image 取 placement 丢 bitmap；紧接 rasterize 再完整 render。ScaleContext 复用不等于缓存该 image.data。非空冷 key 多一遍 raster 与 bitmap 临时分配；warm bounds+atlas 命中不发生。不是 Windows DirectWrite 同一问题。建议单次 cold bounds+bitmap 交付或 bounds-only 能力，避免新无界 bitmap cache。

### R04 — arena/容器历史高水位

`app.rs:889-890,1095-1098`；`arena.rs:150-157,174-204`；`window.rs:981-992,3030-3038`。两个 App arena 初始各 1 MiB，spill chunks 永不 truncate；clear 确实析构对象但保留 chunk/Vec/map 容量。历史 Ce/Cv chunks 保留约 (Ce+Cv) MiB 至 App Drop；双 Frame 和 GPUI 外层 layout maps 亦为峰值容量策略。

正常复用有助低抖动，不是每帧对象漏。建议长时间低水位后软上限/冷却回收超额，而非每帧 shrink；应测 live heap/capacity，不能仅看 RSS。

### R05 — 深 GlobalElementId 路径

`element.rs:234-236,320-335`；`window.rs:2764-2772,4045-4052,4087-4105`。Arc::from(element_id_stack slice) 首次构造复制完整前缀，Hash 也遍历整路径。D 层都有 ID 的单链保留 D(D+1)/2 个 ID 槽及同阶构造/hash 工作；不是 Arc clone 深拷贝。浅树通常线性，状态/帧淘汰会释放，debug inspector 可增加另一份路径。建议共享/interned 前缀、增量 hash，保留命名空间/碰撞校验。

### R06 — 同 view 动画通知未专门合并

`window.rs:2489-2490,2505-2507,1642-1649`；`animation.rs:446`；`transition.rs:147-151`。每个请求 Box 捕获 EntityId，下一帧逐个 notify；同一 view M 个动画/重复采样会有 Θ(M) callback/分配。之后 pending notification 和 dirty 可合并，因此不是 M 次绘制。take+消耗 callbacks Vec 还失去该 buffer 的复用。建议动画专用 EntityId set，与任意用户 on_next_frame callbacks 分开。

### R07 — keymap 查询放大

`key_dispatch.rs:401-445` 对 action 的每个候选重调 bindings_for_input；`keymap.rs:175-188` 每次全 B 条反向遍历，并先算 binding_enabled/context 再匹配输入；`keymap/context.rs:260-266` 可按深度求值。A 个候选的展示查询可 O(ABC)，另有 disabled 过滤/排序/action clone。小 keymap 成本有限，不是历史缓存泄漏。建议先输入/prefix 索引，再算 context，按 KeymapVersion/context 共享结果，保留 Unbind/NoAction/source precedence。

### G05 — Metal 重复 fullscreen shading

`metal_renderer.rs:1387` 提交 Triangle 6 vertices；`gpui_render/src/shaders/filters.rs:155-179` 已使用 fullscreen triangle 的 bit-index UV。顶点 0..2 覆盖 viewport，3..5 同 winding 三角形重复覆盖约半 viewport；scissor 相交域重复 Gaussian/blit shading。输出 replace 可能相同，因此不能靠肉眼无差异排除工作。

这是几何覆盖推算，不是“帧时下降 50%”。DX/WGPU 对应 draw 3；无 Mac capture。建议共享 vertex-count=3 契约并核对所有 pass。

### P01 — 性能别名不可用

`.cargo/config.toml:13-37` 的 perf-test/perf-compare 固定 profile release-fast，当前 workspace/config 没有定义。实际命令 `cargo check -p hgpui --profile release-fast --locked --offline` 返回 `error: profile release-fast is not defined`。本机 hyperfine 也不在 PATH，但那是环境事实，不并成源码 bug。

建议修复/定义正确优化 profile、恢复可运行的回归门。不是 UI runtime 本身的性能损失。

### G06 — 小路径批次的全域 MSAA clear/resolve（C2）

`scene.rs:587-638` 按 painter order/type 拆 batch；DX `directx_renderer.rs:1035-1039,1086-1092`、WGPU `wgpu_renderer.rs:3077-3102`、Metal `metal_renderer.rs:1665-1674` 每 batch 操作 viewport target。小 path 与其他图元交错造成 B 个批次时逻辑操作域 O(BWH)。DX 4x BGRA MSAA+resolve targets 名义 20WH 字节，不等于每次实际搬运这全部字节。

透明/顺序约束不能盲合并；Metal Memoryless 改变外部带宽。建议 capture 区分 clear/resolve/pass成本，考虑局部/分块 targets、resolve 后 discard，保留 AA 与顺序。

### R03 — 晚到动画 dead-ID 通知（C2）

`animation.rs:429-443` 的 max_fps timer detach，仅捕获 EntityId；`app.rs:1749-1753` 先 remove 已释放 ID；`notify:2727-2760` 对任意 ID entry/or_default 再 insert。若短命视图先释放、timer 后在仍活窗口回调，就可恢复一个空 invalidator map，之后不会再经历该 ID 的 Drop 清理。每批不同代数死 ID 可 Θ(K) 增长。

default max_fps=None/reduce_motion/关闭窗口可绕开；同一 ID 重复通知一项；没有动态复现释放/回调顺序。建议 notify 不创造无 invalidator 的空键、WeakEntity/元素取消任务。

### R08 — Entity Drop 的跨线程串行门（C2）

`app/entity_map.rs:339-353` 所有 Drop 先获取全 App upgradable_read，再 fetch_sub，只有最后引用升级写锁。parking_lot 的同一锁只能一个 upgradable reader，因此不同 entity 非最终 Drop 也不能并行进入此处。普通 entity read 已无锁，clone/upgrade 的普通读锁可并行，诊断 tracker 不纳入 release。

争用何时成为瓶颈取决于多后台任务释放量；未测等待毫秒。建议共享读护卫处理非最终释放，再审弱升级/slot 生命周期竞态决定最后释放队列/写锁策略。

### T06 — 动画图过密刷新（C2）

`elements/img.rs:321-345,379-384` 只有 elapsed>=frame-delay 才变帧，但布局阶段每次 active 动画仍 request_animation_frame，无可见 bounds gate。非虚拟化离屏图仍布局，A 张每刷新可能排 A 个通知；不等于同帧重复 GPU 上传。inactive/reduce_motion、无 ID、虚拟化移出布局会排除。

建议按最近帧 deadline 唤醒、可见性/激活门控、合并 wakeup，实际重建范围和帧数需 trace。

### T12 — macOS RTL index 转换（C2）

`gpui_macos/src/text_system.rs:607-618,648-665` 对比前次 UTF16 index，小于则重建 converter，从全文开始扫描。若 CoreText glyph indices 递减 n−1..0，总 char 遍历为 O(n²)。递增 LTR 线性、warm shaping cache 绕开。无 Mac/font/text实际序列核验。建议一次 UTF16↔UTF8 索引表或双向映射。

### G10 — scissor 未约束 logical clear（C3）

DX `directx_renderer.rs:1284-1288,1474-1529`、WGPU `wgpu_renderer.rs:2463-2480`、Metal `metal_renderer.rs:1352-1372,1503-1549` 在 scissor 前整 attachment Clear。每 filter 三次 half-window 全域逻辑 clear；scissor 已减少真正 Gaussian fragment，不能说 scissor 修复没用。

fast-clear 元数据、压缩、tile/memoryless 可能使这很便宜，实际“问题”可能基本不存在。clear 还有防旧 texel 污染的正确性用途。只列待硬件验证，不建议无像素回归就改 Load；局部 scratch/apron 初始化须先证正确。

## 5. S3 低优先级/工具详述

### T13 — SVG 重复 parse

`window.rs:5040-5054`；`svg_renderer.rs:224-236`；`elements/svg.rs:262-273`。新栅格 size 的 atlas miss 又读/parse 同源 Tree，缓存的是结果尺寸 tile，不是parsed Tree。K 新尺寸 O(K×parse成本)，不同尺寸 raster 本身必要。稳定 size 命中无此成本、外部 bytes已有asset共享、SVG字体初始化共享、8192尺寸上限存在。建议带内容版本和预算的 parsed-tree 缓存；未测复杂 SVG 热点。

### P02 — perf duration 整数毫秒截断

`tooling/perf/src/implementation.rs:134-142` 用 mean.as_millis 计算 iter/sec，亚毫秒部分丢失；mean<1ms 时分母=0，得 inf。默认 triage 250ms 可减轻普通场景，固定短迭代/短时间并不因此被数值层拒绝。已有 Display 标 unreliable，但 compare 仍使用截断吞吐率。建议 as_secs_f64、有效时长/非有限结果校验，不把微秒数据用整数毫秒。

### P03 — perf 差异符号说明不一致

`tooling/perf/src/implementation.rs:246-277,393-401,420-445` 注释称 positive=新 run更好、max=最大改善；公式却是 baseline_throughput/new_throughput−1，即新 run更慢产生正值，输出↑。单个 iteration数量相同且新 run两倍快时 delta=−50%，不符合注释的 positive改善语义。算术本身可解释为时间变化，但比较契约/Max/Min改善描述反向，易误读门禁。建议明确统一时间增量还是吞吐率改善，不是运行时性能 bug。

### R10 — 反射额外 AST 往返（C2，编译时）

`styled.rs:19-35`；`gpui_macros/src/derive_inspector_reflection.rs:14-21,188-215,219-295,40-83,129-153`；`styles.rs:1093-1124`。debug/inspector 对已生成样式 TokenStream 以 dummy trait 重 parse，再扫并输出原trait及wrappers，额外 O(T) 处理。只展开认识的宏；rust_analyzer gate及无inspector release排除。源码“10秒”不当测量，最终泛型代码大小未测。建议生成器共享AST/方法元数据，保留反射。

## 6. 实际 Release 基准数据

仅运行已有 `gpui_wgpu/benches/layout_line.rs`，不改源。使用本包固定 font corpus、without_system_fonts、单长显示行（源注释约 3.8–4k chars），直接调用 Cosmic PlatformTextSystem::layout_line，绕开普通 Hgpui LineLayoutCache；所以这是原始 shaping 压力基准，不是静态标签每帧开销或全 UI frame time。

构建：

```sh
CARGO_INCREMENTAL=0 cargo bench -p hgpui_wgpu --bench layout_line --locked --offline --no-run
```

测量两轮：

```sh
CARGO_INCREMENTAL=0 cargo bench -p hgpui_wgpu --bench layout_line --locked --offline -- --noplot --sample-size 40 --measurement-time 5 --warm-up-time 2 --save-baseline hgpui-audit-20261115-e484
CARGO_INCREMENTAL=0 cargo bench -p hgpui_wgpu --bench layout_line --locked --offline -- --noplot --sample-size 40 --measurement-time 10 --warm-up-time 2 --save-baseline hgpui-audit-20261115-e484-repeat
```

| 场景 | 第一轮 Criterion time 估计 ms [区间] | 复测 ms [区间] |
|---|---|---|
| no_fallback | 9.1762 [8.8324,9.6965] | 8.5914 [8.5234,8.6762] |
| with_fallback_ascii | 9.3774 [9.1172,9.6821] | 8.8015 [8.7298,8.8848] |
| mixed_direction_paragraphs | 9.6669 [9.2778,10.106] | 8.8500 [8.7049,9.0050] |

第一轮 40 samples 需要比 5s 更长的估计收集时间（约7.8–8.4s），复测提高到10s并实际收集约14s。各轮有 outliers；不从两轮同版本噪声宣称优化百分比，不把区间称 P95/P99。未做历史上游同 corpus、同toolchain A/B，不将 2026-10 的旧报告数据混为本轮回归基线。

没有自动化运行 GPU FPS/显存压力测试。本机 hyperfine 不在 PATH，perf aliases 的缺失 profile也已实际验证；没有安装工具、清缓存或修改环境配置来绕过。

## 7. 排除/不升级的线索

- mask链已有矩形过滤/单圆角精确折叠、far-inside SDF 快路、fingerprint 跳过未变上传；旧审计的问题不能照搬。
- aa_scale 已预先计算、纯色quad快路、零边条跳过、阴影内部早退等历史修复不再标为未修。
- 容量clear不等于对象泄漏：arena执行对象析构，Frames/GPUI layout maps淘汰旧payload；保留峰值作为R04策略单列。
- LineLayoutCache双帧交换/clear确实驱逐未使用布局；稳定文字不每帧重新shape。
- hash shaping API 有线性扫描潜在二次路径，但仓内无生产外部调用，暂不列常规热点；消费端若使用再专项验证。
- EntityMap普通读取已无锁；不能把handle Drop的串行入口扩大为全链路锁。
- use_keyed_state的退出key随连续帧搬运/清理，未确认所有历史key永久存活。
- next-frame重复通知不等于多次draw；invalidator/notifications会合并。
- Window.destroy no-op 本身不能证明泄漏：Mac dealloc_window/dealloc_view、Linux destroy before surface/window teardown、Web cancel RAF/disconnect observer/take callback/remove DOM路径已核查。
- CMSampleBuffer/CoreVideo RAII retain-release正常；screen-capture裸callback问题不等于每个视频帧漏。
- glyph/SVG atlas无eviction不同于image/shadow移除：三atlas remove确实回收tile，最后key消失后释放整页。
- EditableText历史是按patch、按需、默认1000记录，并非1000整文档快照；极大删除内容本身是undo必要成本，未另凑高危。
- List默认Visible、measure_all有首轮gate；UniformList常数额外测量行、AnimatedList明确非虚拟化不是“意外全量”；AnimatedUniformList有viewport/时长retain。
- Measurer只在size改变时notify；ContainerQuery的FnOnce/当前帧child不造成历史树堆积。
- font/SVG字体DB大初始化、Arc clone、WGPU BindGroup构造、global stylesheet fields本身不能独立证明热点/泄漏。
- benchmark paths_bench故意重复2000条路径并持续动画/复制；不能拿这种负载直接当普通UI回归。
- debug assertions/inspector/leak-detection、rust-analyzer/compiler的CPU/RSS不冒充release应用成本。

## 8. 覆盖、交付和建议顺序

三线静态覆盖：

1. gpui_render 全 shader域/ABI/instances/target stack/filter/blur；Windows DX资源上传/批次/atlas/present/loss；WGPU context/queue/atlas/surface；Metal targets/完成回调/atlas；Linux Wayland/X11与Web资源销毁接线；核心scene/bounds/shadow/paint。
2. App/entity/arena/element/view/window状态与布局、事件/订阅/dispatch/keymap、motion/spring/transitions、taffy/style/grid；scheduler/tokio、collections/sum-tree/path/shared-string/refineable/macros/zed-util关键路径。
3. text_system/layout/wrapper、图片/SVG/asset、EditableText/storage/history/caret、List/UniformList/animated variants、Measurer/ContainerQuery/Transition、Windows/Cosmic/macOS/web文字、media及capture真实帧调用端。

不是逐行形式化穷尽，不对未观察业务频率给精确损失。一个代理首次根目录Glob枚举过宽，但未打开target/第三方内容，随后收窄自有源；报告保留这一审计过程偏差。主线程对owned构建/测量产物的读取用于基准证据。

建议修复/验证顺序（本轮均未实施）：

1. 所有权/失败清理：T01、G03、T14；并单独解决G04确定的DX数据接线。
2. 基础热路径：R01、G01、R02；用业务P/F/N与缓存miss分布量化。
3. 持续资源预算：G02、T02、T03、T04、T05、G09；分别制定CPU/GPU/任务预算。
4. 已知无用GPU allocation/overdraw：G07、G08、G05；与平台capture验证并行。
5. 交互/文本/通知重复工作：T07–T11、R05–R08、T06、T13。
6. 失败/平台时序：G11、R03、R09、T12。G11保留机制已核验依赖，但真实失败调度仍应trace。
7. G06/G10需硬件counter决定收益，不先凭逻辑clear域大就改correctness关键初始化。
8. P01–P03及R10单列为性能验证/编译工具改善，别与runtime收益混算。

后续动态验证应增加：代表性UI的warm/cold frame CPU、布局与paint分解、allocation次数/字节、live heap与RSS、多窗口/重载/失败恢复/大场景退回小场景、GPU资源活对象/上传/clear/resolve/fragment counters。只有这些有数据，才承诺具体吞吐率/内存下降幅度。
