# 渲染管线优化：Gaussian pass 按依赖裁剪

## 目标与基线

用户于 2026-11-28 要求优先降低实际 GPU 工作量、给 UI 留出帧时余量，并要求 Windows DX11、WGPU、macOS Metal 同轮保持一致。实现基线为 `perf/core-elements-hotpaths` 的 `cb95b2e437`。本批不改变采样质量、不删视觉功能、不修改 IshiTone。

最小反馈用实际旧 planner：1920 × 1080 的 viewport、宽 1920 高 50 的 backdrop 标题栏、blur radius 40。在 `blur_region::tests::titlebar_vertical_pass_only_writes_texels_read_by_composite` 中，运行旧代码实际得到 `[0, 0, 960, 85]`，断言需要 `[0, 0, 960, 26]` 失败。根因是 downsample、水平和垂直高斯三个阶段共享同一对称扩边，而垂直 pass 的输出只供最终合成使用。

## 实现与边界

三个后端都调用 `hgpui_render::blur_region::blur_pass_plan`。从最终合成倒推：

1. 垂直 pass 只写合成会读取的双线性采样范围。奇数 viewport 用真实半尺寸 / 全尺寸比例；保留窗口边缘的 clamp。
2. 水平 pass 在垂直输出的基础上只向 Y 扩展。Downsample 再沿 X 扩展，供水平核采样。halo 按实际最远 tap 向上取整并保留一个安全纹素，不使用未取整的理论 3σ 做上界。
3. Backdrop 仍合成到原圆角盒；content blur 仍按原几何外扩，允许模糊溢出。完全不可见的合成可跳过 Gaussian pass。原 tap 数、取样位置、颜色矩阵、mask chain、blend、清屏策略及 shader ABI 未改。

| 目标场景的半分辨率 scissor 面积 | 旧 | 新 |
|---|---:|---:|
| Downsample | 81,600 | 83,520 |
| 水平 Gaussian | 81,600 | 83,520 |
| 垂直 Gaussian | 81,600 | 24,960 |
| 两阶段 Gaussian 合计 | 163,200 | 108,480（−33.5%） |
| 三阶段合计 | 244,800 | 192,000（−21.6%） |

Downsample 和水平 pass 多留 2 行覆盖精确采样及保护带，这是为像素正确性保留的成本。面积是光栅工作区，不等于硬件计数器、GPU 总占用或显存；全窗模糊不能靠这项变化减少同等比例工作。没有修改 scratch 尺寸和 allocation 生命周期。

## 实机数据与验证

DX11 实卡为 **AMD Radeon RX 580 2048SP**。同进程 GPU timestamp 中位数：旧 **0.15630 ms**、新 **0.13671 ms**，每次滤镜链节省 **0.01959 ms（12.5%）**。7 组样本分别落在 0.15468–0.19073 ms 和 0.12514–0.15277 ms。这证明目标场景的 GPU 工作时间下降，但不能据此宣称整窗 GPU 占用下降 12.5%、帧率上涨或肉眼可见变化。

WARP 四组逐像素对照均与全范围参考一致。DX11 原生全测 **49 通过 / 2 忽略**（其中硬件基准已单独运行）；共享渲染契约 **44 通过**；独立 WGPU shader validation **1 通过**，Windows `wgpu` feature 和 Windows 宿主 workspace `--all-targets` 编译检查通过。最初一次硬件夹具未校验每个查询的 `S_OK` 并排除了合法的零起始值，报无效 timestamp；修正夹具后完成上述实卡测量，没有用 CPU 时间代替。

### 复跑命令

```sh
export CARGO_INCREMENTAL=0
cargo test -p hgpui_render --lib --locked --offline
cargo test -p hgpui_windows --lib --locked --offline -- --test-threads=1
cargo test -p hgpui_windows --lib --locked --offline hlsl_blur_pass_scissors_hardware_timing -- --ignored --nocapture --test-threads=1
cargo test -p hgpui_wgpu --test shader_validation --locked --offline -- --test-threads=1
cargo check -p hgpui_windows --features wgpu --locked --offline
cargo check --workspace --all-targets --locked --offline
```

- WARP 回读夹具用真实 DXBC 的 downsample / 两个 Gaussian / 合成，逐字节对比全范围参考与分 pass scissor，scratch 用亮色 sentinel 暴露漏算的纹素。覆盖 257 × 191 奇数 viewport、分数边界、遮出 viewport、backdrop 与 content filter、极小与跨 32 tap 上限的 blur。
- DX11 实机夹具在同一进程交替执行旧/新 scissor，使用 GPU timestamp（非 CPU 提交计时）。两个策略使用相同纹理、shader、清屏和合成。每样本 16 次同场景，预热后按 7 轮中位数报告；这是滤镜链，不包含 UI 构建、swapchain Present 或帧同步。
- macOS 的函数入口、pass 顺序、全/半 viewport、uniform、scissor 绑点及 composite 已同轮同步；没有在 Mac 上编译或实机验证。
