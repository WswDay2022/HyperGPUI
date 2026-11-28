# 默认 Release 诊断开销检查

基线：`b70dabd49d`，Windows MSVC、Cargo 1.98.0、默认 Cargo Release、锁定依赖。目标是排除非主动启用的诊断工作，不通过关闭功能、错误日志、GPU 正确性校验或改变应用发布配置来缩小结果。

## 检查范围

- **无障碍功能与调试快照分开。** 旧实现开启 AccessKit 后，每个已绘制帧除正式 `TreeUpdate` 外，还扫描 tab stops、深复制树、取得本地时间并格式化时间戳，供 `debug_a11y_tree_json()` 读取；这不是系统读屏器所需的数据。
- **DX11 批次标签。** 旧实现即使 ETW 没有启动也会为成功绘制的每个批次构造 Rust `String`；HSTRING 编码原本已按 ETW 启用状态门控，Rust 部分没有。
- **有意保留。** DX11 调试层探测已经由 `debug_assertions` 阻断；WGPU 原生实例不主动启用调试层，保留显式环境变量覆盖与 indirect-call 校验。保留错误日志、设备丢失恢复、断言中的运行时安全设计以及主动启用的帧追踪。WGPU 的 `frame_stats()` 是公开统计接口；是否改为显式采集应先核对调用方与测量，未静默置零。测试/bench 代码不是普通 Release 应用的发布路径。

## 默认 Release 契约

- `debug_a11y_tree_json()` 的签名不变，默认 Release 返回 `None`。Debug 或显式启用 `hgpui/inspector` 仍有快照。用测试平台接收真实 `TreeUpdate`，验证同一按钮节点、Focus 和 Click 不依赖调试快照；无需安装系统屏幕阅读器就能验证该层契约。
- DX11 成功且 ETW 关闭时不构造批次标签。ETW 活跃时仍生成原标签，错误上下文仍保留批次名称；不按编译配置禁用真实诊断入口。
- IshiTone 当前发布 profile 已指定 `debug = false`、`strip = "symbols"`、`opt-level = "z"` 和 LTO；这些符号/代码大小设置不能替代去掉仍被调用的诊断。本轮保持 IshiTone 不变。

## 复跑命令

```sh
export CARGO_INCREMENTAL=0
cargo test --release -p hgpui --lib --locked --offline cached_view_keeps_accessible_actions_after_parent_redraw
cargo test --release -p hgpui --lib --features inspector --locked --offline -- accessibility_snapshot_benchmark cached_view_keeps_accessible_actions_after_parent_redraw --include-ignored --nocapture --test-threads=1
cargo test --release -p hgpui_windows --lib --locked --offline -- disabled_batch_trace --include-ignored --nocapture --test-threads=1
cargo build --release -p hgpui --example hello_world --locked --offline
cargo check --release -p hgpui_windows --features wgpu --locked --offline
```

快照基准只测同样树的 finalization 和诊断捕获，不含元素重建、实际 OS 投递或 UI 帧时；批次标签基准只测 CPU 诊断开销，不含 GPU 绘制或 ETW 已开启的情况。

## 已验证结果

无障碍 finalization，256 个内容节点、每轮 100 帧，7 轮交替运行取中位数：保留诊断快照 → 仅保留平台更新，分配请求 **51,800 → 200 次 / 100 帧**，请求字节 **12,658,600 → 7,425,600 B / 100 帧**，每帧 **98.63 → 9.06 µs**。默认 Release 核心库 **489 通过 / 6 忽略**；显式 Inspector 的 Release 功能回归和基准均通过。保留的 200 次请求属于该夹具的正常 TreeUpdate，不能称为整应用零分配。

DX11 未启用跟踪的 CPU 夹具，10 万个成功批次、7 轮交替取中位数：构造标签 **100,000 → 0 次**，**338.11 → 0.86 ns/批**。Windows 原生 Release 库 **52 通过 / 5 忽略**。普通 Release 的 `hello_world` 实际重新构建成功，Windows WGPU Release 编译检查通过。没有做精确的旧/新整应用产物体积对照，也未在 macOS/Linux 上编译或运行。
