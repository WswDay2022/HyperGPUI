# 渲染管线：消除可证明的冗余工作

本轮从 `4fede0b89e` 出发。用户的 UI 本来流畅；目标是以同等视觉输出和呈现节奏花更少的 GPU/CPU 工作，而非复现一个不存在的卡顿。不改 IshiTone，不启动整应用测试，不把名义纹理字节或 draw 次数当帧率测量。

## 管线巡查

| 阶段 | 源码事实、接受门与取舍 |
|---|---|
| 场景整理与重放 | `Scene::finish` 产生分类型有序批次，裁剪链已有索引和指纹。隔离组应按实际批次标记的同时存活数分配，不按总组数。不能为合批重排重叠图元；缓存重放保序。 |
| Render target 分配 | 全尺寸 root、隔离内容组、backdrop 色彩快照、半尺寸 Gaussian scratch 是不同用途。先证明 GPU 实际路径会消费哪个资源，再分配；保留 resize/device-loss 释放和热复用。 |
| 路径栅格化 | DX11 `ResolveSubresource`、WGPU resolve attachment、Metal `MultisampleResolve` 目前是全尺寸路径中间目标；每个 path batch 的 ROI、MSAA 清除/resolve 成本尚须硬件计数或计时，不能因目标大就去掉清除或改变采样。 |
| Gaussian | `4fede0b89e` 已按读依赖裁剪三阶段，窗口边缘采样与 mask 不变；本轮避免给纯非 Gaussian 场景申请 ping/pong，而不是降低模糊质量。 |
| 合成与呈现 | 只有 backdrop 会读此前的目标；content filter 读自己的 group，是否需要全窗 offscreen + blit 按根目标读取依赖判断。全屏和矩形的 shader 顶点契约不同。保持窗口损伤、透明度、Present 和刷新策略，不通过限帧减少数字。 |
| 实例与绑定 | WGPU 多处在批次创建 bind group。是否能跨批复用取决于 offset、页身份、resize/device-loss 生命周期；未用猜测性缓存替换。优先以固定批次数和驱动开销对照建立反馈。 |

## G07 — 顺序隔离组复用目标

确认旧实现三个后端均以 start 总数封顶 2，而到达实际 draw 时按当前隔离深度索引。共享红测：`start,end,start,end` 旧结果 2，预期 1。实际标记顺序扫描后：顺序 1、嵌套 2、三级仍封顶 2、无边界 0。WARP `GetDesc` 证明分别申请 1/2 张 384×216 BGRA8 的真实资源和对应 RTV/SRV；4K 条件下少一张名义 **31.64 MiB** 的目标。保留已热用过的池容量，不承诺此项立刻回收先前真正需要的峰值页。独立提交 `7a3ca72d3b`。

## G08 — Gaussian scratch 与普通目标拆开

原生红测在只申请 root 后发现 ping 已创建。拆开后 WARP 观察到 root 存在、ping/pong 均不存在；在同一资源实例首次启用真实模糊时创建两张正确半尺寸纹理，并复用原 root。共享测试检查正半径 backdrop/content、纯色彩、纯裁剪和超深度 inline，Metal 的纯色彩 content 合成不再依赖 ping/pong 存在。4K 的名义差为 **15.82 MiB**，只统计从未使用高斯 scratch 的冷态/重建期；不会在冷热帧间每帧销毁和重建。独立提交 `27ce81a927`。

## 仅内容组 — 去掉根中转和全窗 blit

根可采样性、Gaussian scratch 和隔离池独立判断。测试先以同一内容模糊组复现旧逻辑错误要求根纹理；改后纯内容过滤不创建根纹理、不发最终 blit，并可照常启用组与 Gaussian 资源。只要存在 backdrop，保守地走原可采样路径。三端根目标 / 组进入退出接线同步；本机共享 planner、原生模糊回读、Windows wgpu feature 通过。独立提交 `514fcdc4b1`。

收益是**符合条件时每帧少 1 次全屏绘制**，4K 对应 8,294,400 个名义覆盖像素，以及冷态少 1 张 **31.64 MiB** BGRA8 根纹理；不等于实测带宽、现存缓存回收，或适用于仍启用 backdrop 的窗口。保留组内合成、清屏、颜色格式和现有 Present。

## G05 — 全屏三角和矩形提交各归共享契约

共享 shader 的 fullscreen 入口只用 3 个顶点；矩形 `rectangle_corner` 用 vertex ID 的低两位得到四个角，其拓扑是 4 顶点 triangle strip。Metal 旧实现统一发 6 顶点 triangle list：fullscreen 多提交一个三角，合成和视频 Surface 的矩形还会重叠/漏覆盖。WARP 使用同一个共享合成 shader，重现旧提交在 (18,58) 为 alpha=0；使用共享契约后该点回读为预期完整红色。不是 Mac 实机回读。Metal 下采样另需用全尺寸来源纹理定义 UV，不能用半尺寸 raster viewport 归一化；DX/WGPU 原本已经绑定全尺寸场景。

三端 blur 提交采用共享 `Pipeline::fixed_draw` 定义，Metal 矩形 Surface 也按同一元数据执行。未改 shader、像素格式及混合状态。独立提交 `1b7089456b`。

## WGPU — 连续可见批次借用原数据

对实际渲染调用的 `visible_items` 建立操作计数：64 个全可见项，旧实现 **128 次 bounds 判定、64 次 clone**，红测失败。新实现一次扫描；若全不可见或可见项连续，借用空切片/原始子切片，实际转绿 **64 次判定、0 次 clone、无临时 Vec**。非连续时按原顺序聚集，只为确实可见的项分配容量，第二遍收窄到首尾可见项之间。覆盖了首尾裁剪、带洞的可见区和全不可见三种边界。DX11/Metal 不调用这个 WGPU 特有的过滤器；没有人为加入跨平台克隆或改裁剪谓词。

## 量化边界和下一轮测量

若一帧只有至少两个顺序的裁剪组、无 backdrop、无 Gaussian，并且缓存处于冷态，这三项资源规则叠加让 4K BGRA8 名义分配由约 **110.74 MiB 降至 31.64 MiB**，同时少一次最终全窗 draw。这个数字只计算相关纹理，不是进程总存储或驱动驻留显存。

共享渲染契约 **47 通过**，DX11 原生 **50 通过 / 2 忽略**；WGPU 可见性专项 **2 通过**，独立 shader 验证 **1 通过**。Windows `wgpu` feature、Windows 宿主 workspace `--all-targets`、Metal 源文件静态语法检查通过。未在 Mac 上编译、运行或测量；未启动 IshiTone，未新增整应用帧时/占用结论。

后续优先给路径 MSAA 建立「极小路径 × 多批、4K viewport」的 GPU clear/resolve 基线，区分快速清除元数据与真正的写入/resolve 开销；同时测 WGPU 批次 bind group 创建量。暂不修改无证据的呈现节奏、清屏和资源退休。

```sh
export CARGO_INCREMENTAL=0
cargo test -p hgpui_render --lib --locked --offline
cargo test -p hgpui_windows --lib --locked --offline -- --test-threads=1
cargo test -p hgpui_wgpu --lib --locked --offline wgpu_renderer::visibility_tests -- --nocapture --test-threads=1
cargo test -p hgpui_wgpu --test shader_validation --locked --offline -- --test-threads=1
cargo check -p hgpui_windows --features wgpu --locked --offline
cargo check --workspace --all-targets --locked --offline
```
