# 性能审计第 1、2 组修复记录

## 范围与基线

- 仅处理 T01、G03、T14、G04；没有开始第 3–5 组。
- 基线：`e484b5d6d4`；修复分支：`fix/perf-ownership-render-wiring`。
- 每项单独本地提交，没有 push；不改 IshiTone 的未提交工作。

## T01 — TextSystem wrapper 池强引用环

### 发现与最小复现

池保存 LineWrapper，wrapper 保存 Arc<TextSystem>，形成 pool → wrapper → owner 环。
新增三项回归：归还 wrapper 后最后外部 owner 能释放、最后活跃 lease Drop 后能释放、缓存复用不留下父 Arc。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib line_wrapper_ --offline --quiet
```

修改前实际 3/3 失败；弱引用仍可 upgrade，归还池后 Arc strong_count 为 2 而非 1。

### 修复与验证

- wrapper 的强父引用仅在活跃借用期间挂接；归还前清除。
- handle 继续持有 TextSystem，确保归还池及解锁之前 owner 不析构。
- 字符宽度缓存及字距复位行为保留；未加入逐字符 Weak::upgrade 的成本。
- 修改后核心 427 项测试通过。
- 提交：`3d4a82bbf7`。

## G03 — 失败 atlas 页不发布

### 根因与修复

旧顺序是 clamp/创建/发布新 texture，然后用原始需求做首次 tile allocate；失败留下零引用页，free_list 也可能被提前消耗。

- 三端先校验原始需求：零/负、超过后端实际上限的尺寸直接拒绝，不扫描或创建页。
- DX/Metal 仍使用原有 16384 上限；WGPU 使用当前设备 max_texture_dimension_2d。
- 共用 AtlasTextureList::try_insert_with：peek ID，局部创建资源并完成首 tile 分配，成功才发布 slot/消费 free_list。
- 首分配/视图创建失败由 RAII 释放临时 GPU 资源，列表与 free_list 不变。
- 只有默认 1024 页可以缩到设备上限；不缩小原始 tile 需求。

### 回归与证据边界

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib platform::atlas_tests --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_windows --lib directx_atlas::tests --offline -- --test-threads=1 --nocapture
CARGO_INCREMENTAL=0 cargo test -p hgpui_wgpu --lib wgpu_atlas::tests --offline -- --test-threads=1 --nocapture
```

- 核心 3 项、DX atlas 3 项、WGPU atlas 6 项通过（包含原有测试）；小型 GPU 测试实际执行，没有不可用 skip 输出。
- 新回归覆盖重复失败、新 slot 与复用 slot、零资源增长、正常 ID 复用；失败注入使用小页，不进行巨大显存耗尽。
- Metal 新增同类测试并静态核对，未在本机运行 Mac。
- 并行实现先完成，因此本项没有执行修改前 red 测试；不把修改后通过当作已观察到旧故障的运行证据。基线代码顺序与事务失败测试共同锁定清理缺口。
- 提交：`b5c93d241f`。

## T14 — macOS ScreenCapture output 回调所有权

- 注册 output 的 dealloc，先置空 ivar，再以准确的 `Box<Box<dyn Fn(ScreenCaptureFrame) + Send>>` 所有权释放环境，最后调用 NSObject dealloc。
- 每帧只借用 callback，不再 from_raw 后 forget；调用期间 retain output，防止合法重入关闭 stream 提前释放环境。
- 正常 Drop、addStreamOutput 失败、startCapture 失败仍通过既有 ObjC release 路径，最终 output 析构统一回收。
- 新增三项 Mac 回归：最后 release 恰好 Drop 一次、借用/非最终 release 不提前 Drop、空 ivar 可析构。
- Windows 本机仅做 rustfmt 语法及 ownership/ivar/super/release 路径静态核对；没有 Mac 编译、测试或 ScreenCaptureKit 异步实机验证。
- 提交：`9ad6e0ab60`。

Mac 验证入口：

```sh
cargo test -p hgpui_macos screen_capture::tests -- --nocapture
cargo test -p hgpui_macos metal_atlas::tests --lib
```

## G04 — DX smoothing 数据接线（独立正确性修复）

### 正确反馈 seam

先将生产 upload/draw 逻辑无行为变化地集中到 DirectXRenderPipelines 的内部 helper，WARP 回归使用同一 helper，不像旧 shader 测试那样手动给 smoothing buffer 上传。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui_windows --lib smoothed_scene_upload_ --offline -- --test-threads=1 --nocapture
```

修改前实际观察到两项失败：

1. 一个 smoothing quad 的中心像素为 `[0,0,0,255]`，没有读取上传的红 quad。
2. 64 个普通 quad 后一个 smoothing quad：`first=64 count=1 stride=304 buffer_elems=64`，SRV 返回 `0x80070057`。

### 修复设计

- 每种图元只保留一份由生产 upload 更新、能够增长的实例 buffer/SRV。
- smoothing 只保存额外的 vertex/fragment ShaderStages，不再创建或保留独立实例 buffer。
- 正常与 smoothing 的 range 都从当前共享 buffer 建立，保持全帧 start/count 语义；blend、ABI、draw order 不改。
- 回归覆盖小场景真实像素和增长后的 quad/shadow/poly 全帧偏移，同时保留已有圆角 smoothing shader 像素比较。

- 两项生产 smoothing WARP 回归已从失败转通过；包含 quad/shadow/poly 的增长后全帧偏移。
- 既有 `hlsl_corner_smoothing_readback` 像素比较通过。
- 提交：`2c7a8d78b9`。

## 最终本机验证

- 核心：427 passed。
- Windows：46 passed / 1 ignored；ignored 是会打开字符映射表 GUI 的既有测试。
- WGPU shader_validation：1 passed；WGPU atlas 6 项已在独立过滤、单线程调用中通过。
- 组件库：142 passed。
- 渲染契约：43 passed。
- `cargo check --workspace --all-targets --offline`：通过。
- Windows wgpu feature 编译检查：通过。
- 所有补丁 `git diff --check`：通过。

T01 与 G04 的新增反馈回归已实际观察修改前失败和修改后通过；G03 的修改前 red 未运行，保持前述证据边界。T14 与 Metal atlas 的 Mac 回归没有在本机执行，不能标为已 Mac 实机验证。没有启动 IshiTone 或修改其未提交源码，没有实施第 3–5 组优化。
