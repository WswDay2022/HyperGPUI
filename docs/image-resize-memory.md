# 图片背景与 Windows 调整窗口大小的内存排查

## 现象与复现

IshiTone 通过 `NewWindow` 启动，实际加载的是 1920×1080 JPEG（`ObjectFit::Cover`，视差关闭）。它同时开启根 `clip_group`，而标题栏的 backdrop acrylic 随“背景图启用”一起开启。直接关掉背景设置并不是只改变图片变量。

在 Windows 原生 DX11 最小窗口复用这张 JPEG；用 `SetWindowPos` 分 20 步从 800×500 调到 1600×1000，再一步缩回，待机后重复一次。分别观察私有提交内存（`PrivateMemorySize64`）和总工作集（`WorkingSet64`），这两项都不等于显存。夹具不启动 IshiTone，不联网，不读取其配置。旧版 `3f6c94979f` 的空窗、仅图片和仅阴影对照缩回后的私有提交变动均不到 1 MiB；仅隔离裁剪约 +26–33 MiB，仅背景模糊约 +36–54 MiB，图片、剪裁、阴影和模糊同时开启约 +74 MiB。这个夹具中**总工作集没有同幅上升**，不能将该数值等同于用户看到的任务管理器读数。

## 定位

- 相同 `ImageSource` 命中同一个解码资源；atlas 的 key 是 `image_id + frame_index`，不是布局尺寸。`Cover` 的可见子 tile 是原有 tile 的取样坐标，不会每次申请新 atlas 页。
- DX11 按窗口尺寸创建场景、滤镜和隔离纹理。原路径先执行 `ResizeBuffers` 和重建交换链视图，然后才释放旧尺寸的中间资源引用。只提前 drop 能降低峰值，但私有提交的高水位仍滞留；每次 resize 强制 `Flush` 也未消除滞留。提前释放后用一次 `IDXGIDevice3::Trim`，可释放大量已不用的驱动分配。这不是无限增长的图片缓存泄漏。
- 修复在 `ResizeBuffers` 前解绑并释放旧中间资源，现有滤镜、裁剪、atlas、DPI、采样和呈现策略保持不变。仅有实际退休的中间资源才启动一次短定时器，resize 活跃时推延，最近尺寸变化至少 250 ms 后才 Flush/Trim 一次。没有额外绘制帧、逐事件 Flush 或在热路径等待 GPU；如设备不提供 `IDXGIDevice3`，仍正常调整窗口。WGPU 和 Metal 已有独立的尺寸资源失效路径，没有移植 DXGI 设备操作。

## DX11 修复前后

同机、相同 JPEG 和 800×500 → 1600×1000 → 800×500 的两次循环；以下是缩回后等待 2.5 秒与初始小窗口比较的私有提交增量。旧值为同机探针关闭时的采样；修复后使用定时器版本，尚未加入独立的 JPEG 解码优化。

| 最小窗口内容 | 旧路径 | 提前释放并在 resize 停止后 Trim |
|---|---:|---:|
| 仅隔离裁剪 | +32.90 MiB | +0.20 MiB |
| 仅背景模糊 | +36.08 MiB | −0.01 MiB |
| JPEG、隔离裁剪、模糊、阴影 | +74.16 MiB | −8.22 MiB |

最后一行实际数据为旧 49.13 → 123.29 MiB，新 49.26 → 41.04 MiB；负值表示首次 trim 也释放了启动时仍在私有提交中的空闲驱动分配，不表示活跃图片资源被移除，也不承诺每个驱动都精确回落到同一数值。调试构建的最小窗口证据不外推整应用 Release、帧率或显存。

Windows 原生库回归（含 WARP）为 **51 通过 / 4 忽略**。本机复跑夹具留存在工作区 `tmp`，无生产调试开关：将 `hgpui-image-resize-memory.rs` 临时放入 `crates/gpui/examples/image_resize_memory.rs`，然后 `cargo build -p hgpui --example image_resize_memory --locked --offline`。用 `hgpui-image-resize-memory.ps1 -Exe <binary> -Image <jpeg> -Cases 'c,b,ibcs' -IdleMs 2500 -MaxRetainedMiB 15` 驱动新旧二进制，分别要求基线复现、修复后通过。阈值用于本机对照，不作为跨 GPU/驱动的通用预算。
