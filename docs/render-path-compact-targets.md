# G06 — 小路径批次使用局部 MSAA / resolve 目标

前置[硬件基线](render-path-msaa-baseline.md)确认 RX 580 的瓶颈是大附件 resolve，不是 clear。本轮没有跳过清屏、改变 MSAA 采样数，或只在全尺寸附件上设置 scissor。

## 设计与范围

- `PathTarget` 按每批**原始三角形包围盒的并集**与 viewport 选择 `128×128`、`512×512` 或原全窗目标。向外取整并留一纹素护边，起点向下对齐到 2 px，维持解析曲线的导数 quad 相位。只在小目标面积至多为全窗一半时走紧凑路径；过大的并集、异常坐标和分散路径保留全窗回退。
- 三端按桶懒创建附件，批间复用，resize/device-loss 沿用各端原失效语义。不会每批分配；DX11/WGPU 不再为只有小批次的窗口预留全尺寸 resolve/MSAA，Metal 保留 Apple GPU 上原有的 memoryless 选择。4× BGRA8 的纯 128 桶名义 **0.3125 MiB**；两个小桶都用过时额外容量封顶 **5.3125 MiB**，不含必要时的全窗回退、驱动元数据或其他纹理。
- 栅格位置和纹理 UV 在局部目标坐标系中计算，渐变、矩形裁剪和 mask chain 仍使用窗口坐标；路径 sprite 仍按原顺序及覆盖规则合成。栅格顶点复用原 12 B padding（仍 **144 B**），sprite **16→32 B**；DX11、WGPU、Metal 使用同一 Rust ABI 和 Naga 生成的着色器接口。

## 像素与实卡证据

- WARP 以原全窗路线作参考，用真实栅格和 sprite 合成 shader 对照整张 **965×677** BGRA8 输出。覆盖非零坐标、分数边缘、半透明渐变、曲线、圆角/fade mask、重叠批次、窗口边缘和 `128→128→全窗→128→512` 的复用/切换。正式断言逐通道差≤1，实际最大差 **1/255**，不是字节完全相同。
- 排查中，按**已裁剪**范围分配曾在分数 mask 边缘出现一个像素通道差 4；只保留偶数导数相位并未消掉。改按原始三角形范围规划，避免在现有蒙版内引入新的附件裁剪，才回到差≤1 的验收门，没有放宽断言。
- RX 580 2048SP / DX11，用新坐标映射实测同一条远离原点的小路径，4K 逻辑 viewport、4× BGRA8、24 批每样本、七轮交替顺序中位数。清除＋栅格＋resolve 从 **0.26279 ms/批降为 0.02061 ms/批**；独立复跑为 **0.27235→0.02207 ms/批**，两次均约 **92%**。纹理分配、主场景合成、窗口 Present、输入和 UI 帧时不在这组数据里；没有外推到 WGPU 或 Metal。

## 后端验证

Windows DX11 原生 **51 通过 / 4 默认忽略**，本轮新增的实卡项已单独跑通；共享渲染契约 **49 通过**。WGPU 独立 shader 验证、Windows `wgpu` feature、workspace `--all-targets` 检查通过。Metal 接线和共享生成接口已同步核对，未在 macOS 上编译或实机运行；未运行 IshiTone，不主张整应用帧时收益。

### 复跑

```sh
export CARGO_INCREMENTAL=0
cargo test -p hgpui_windows --lib --locked --offline -- --test-threads=1
cargo test -p hgpui_windows --lib --locked --offline compact_path_batch_hardware_timing -- --ignored --nocapture --test-threads=1
cargo test -p hgpui_render --lib --locked --offline
cargo test -p hgpui_wgpu --test shader_validation --locked --offline -- --test-threads=1
cargo check -p hgpui_windows --features wgpu --locked --offline
cargo check --workspace --all-targets --locked --offline
```
