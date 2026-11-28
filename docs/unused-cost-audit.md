# 未使用功能的开销审查

审查基线：`b54d75ce66`。先查实际默认路径上的资源创建、每帧工作与每图元记录；未启用的诊断、光栅效果和加载器不应为只是“可用”而付费。不能把基础绘制、正确性校验、平台功能注册或明确的用户功能当作无用开销。数字均限于夹具或资源描述，不外推整窗帧率、驱动驻留显存或 RSS。

## 本批验收

| 位置 | 旧默认路径 | 修正与启用后的语义 |
|---|---|---|
| 共享 SVG 加载 | 构造 `SvgRenderer` 即创建 usvg options、空 FontDB 和 font resolver，即使只载位图或从未解析 SVG。旧回归实测 14 次 Rust 分配请求、691 B。 | 只留可克隆的共享句柄；第一次解析 SVG 才构造一套 options，clone 不分配，字体数据库仍仅在需要解析文本时加载。原 BGRA 输出和样式分辨率不变。 |
| WGPU 渲染统计 | 无调用方也更新每批、每类图元、每个 GPU draw 等计数，并在每帧或重试时清零。 | `hgpui_wgpu/frame-stats` 显式选择，默认不编译计数字段、写入和表达式。需要 `WgpuRenderer::frame_stats()` 的调用方启用该 feature；开启后的字段及统计范围不变，不用虚假的全零值冒充完整统计。 |
| 滤镜参数 GPU 缓冲 | DX11 在创建管线时申请常量缓冲，WGPU 在创建窗口资源时预留 256 个动态对齐槽，即使从不绘制滤镜。 | DX11/WGPU 第一次真实滤镜/合成/blit pass 按原描述申请，同一设备上后续 pass 复用。DX11 分配失败仍能重试；WGPU 仍使用原先的对齐、frame slot 与 spill 逻辑；设备资源重建时随原 owner 销毁。Metal 原本就在 pass 内通过 `set_fragment_bytes` 提供参数，没有对应的常驻参数缓冲。 |

## 已测结果

- **SVG**：Release、同进程每轮构造 10,000 个不解析 SVG 的 renderer，7 轮交替中位数。分配请求 **140,000 → 10,000 次**、**6,910,000 → 480,000 B**；**2901.64 → 147.26 ns/renderer**。剩余 1 次 / 48 B 是共享句柄和惰性状态；不是声称整个 renderer 已零分配。7 项 SVG 测试/基准通过。
- **WGPU 帧统计**：每个成功绘制的普通 quad 批次触发三次统计更新（总批数、quad 数、draw 数）。同一统计宏的 10,000 批次写入夹具：默认 **30,000 → 0 次**；显式启用 `frame-stats` 后 **30,000 次**，未改变统计语义。
- **滤镜参数**：本机 WGPU device 的创建描述 **65,536 → 0 B/冷 renderer**，DX11 WARP 用 `GetDesc` 实测 **208 → 0 B**。第一次实际调用参数分配入口创建原尺寸缓冲，第二次获得同一对象，参数写入通过。未将名义分配量算作实际显存或 CPU 工作集；没有拿 shader 启动时间的收益冒充参数缓冲的收益。
- 本机最终回归：核心 Release **490 通过 / 7 忽略**；Windows DX11 **53 通过 / 5 忽略**；WGPU 独立 shader validation **1 通过**。Windows 宿主 workspace `--all-targets` 和 Windows `wgpu` feature 编译检查通过。

## 审查边界与后续候选

- **着色器管线仍是下一项。** 三端启动时会创建 corner-smoothing 专用管线/着色器和滤镜 shader。延迟实际 GPU 管线创建能进一步削减普通窗口的启动工作，但必须测首个特效帧、重建设备、首次出错回退及三端像素一致性。本次只修参数缓冲，不把 shader 本身标已优化。
- **普通矩形遮罩仍进原始遮罩栈并被软裁剪链筛选。** 若推进，应验证无圆角/fade 时不扫描祖先，保留有效矩形交集、Deferred 上下文和路径裁剪；优化前后做深栈操作计数和像素/命中回归。
- **阴影维护尚未纳入本批。** 普通窗口也维护少量帧状态；启用并缓存大量阴影后另有重放扫描。后者是使用了效果时的热点，不能用停止清理缓存来优化，也不能混成未使用开销的结果。已知滤镜 spill 与缓存预算问题的上限/回落不在本批修改中。
- 平台事件注册、无障碍激活回调、设备错误恢复、实际基础字体/位图/布局功能均保留；没有把“未启用诊断”扩张为禁用正常平台功能。Windows 有运行条件，macOS/Linux/Web 的运行行为不能由 Windows 编译推断。

## 可复跑入口（Git Bash，仓库根）

```sh
export CARGO_INCREMENTAL=0
cargo test --release -p hgpui --lib --locked --offline -- svg_renderer::tests --include-ignored --nocapture --test-threads=1
cargo test -p hgpui_wgpu --lib --locked --offline -- frame_stats_tests filter_uniforms_allocate_only_on_first_pass --include-ignored --nocapture --test-threads=1
cargo test -p hgpui_wgpu --lib --features frame-stats --locked --offline -- frame_stats_tests --include-ignored --nocapture --test-threads=1
cargo test -p hgpui_windows --lib --locked --offline -- filter_uniforms_allocate_only_on_first_pass --nocapture --test-threads=1
```
