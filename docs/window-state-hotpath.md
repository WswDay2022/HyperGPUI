# Window state：命中复用已有路径

`Window::use_state` 用调用位置作为 key，再进入 `use_keyed_state`。旧版即使命中，也会把整个 ID 栈复制进新的 `Arc<[ElementId]>`，并在本帧查找、上帧查找与写回时分别计算 key 哈希。复制非整数 ID 还可能增加引用计数操作。

新版借用 ID 栈，只计算一次哈希供两张状态表的查找和写回复用。命中后移出并复用已有的 owned key 和状态盒；只有使用 ID 栈且真正未找到 key 时才构建新的路径。哈希只用于定位，匹配仍逐个比较路径 ID 和 `TypeId`。`with_element_state` 走同一张表，调用方传入的 `GlobalElementId` 格式与 API 不变。回调仍在没有借用状态表时执行；缓存重放仍由同一份效果范围日志维持状态寿命。上帧状态全部移走后，帧尾不再继续逐键探测空表。

## 验收口径

每行各调用一次 `use_state` 和 `use_keyed_state`，用 `TestAppContext` 驱动实际窗口绘制。提前预热两张状态表和访问日志；只测 hook 批次，不包含布局、绘制、帧尾重放或首次建状态。分配数据是同线程的 Rust 分配请求，不是驻留内存。

Windows x64 / Rust 1.98.0，基线 `4d3c959275` 的隔离快照和新版使用逐字相同的夹具。Release 预热后每进程采 121 个批次，`A B B A A B` 独立运行后的进程中位数如下：

| 每帧行数 / 额外命名空间层数 | 每帧 hook 调用数 | 分配请求（旧 → 新） | 请求字节（旧 → 新） | CPU 耗时（ns/次，旧 → 新） |
|---|---:|---:|---:|---:|
| 32 / 0 | 64 | 64 → 0 | 8,704 → 0 B | 393.75 → 256.25（−34.9%） |
| 512 / 0 | 1,024 | 1,024 → 0 | 139,264 → 0 B | 512.30 → 345.51（−32.6%） |
| 512 / 8 | 1,024 | 1,024 → 0 | 466,944 → 0 B | 939.75 → 476.95（−49.2%） |

零分配回归在旧版 128 行时首先得到 `256 次 / 34,816 B`（红），新版浅层、深层都转绿。另两条回归覆盖同帧与跨帧复用、不同命名空间和类型的隔离、初始化器可见的 ID 栈、与 `with_element_state` 的互通，以及真实缓存重放、通知和移除后的释放。核心测试 **483 通过 / 6 忽略**，workspace `--all-targets` 检查通过。改动位于共享 CPU 路径，不依赖渲染后端；未测整应用帧时。

复现命令（Release 基准与默认回归都使用真实 `Window`，但不启动平台窗口）：

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --locked --offline state_hooks
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --release --locked --offline state_hooks_release_benchmark -- --ignored --nocapture --test-threads=1
```
