# 2026-11-24 至 2026-11-28 劳动成果验收

## 范围和基线

- Hgpui 本地分支 `perf/core-elements-hotpaths`，验收起始 HEAD `4796b71dbb`；以 UTC+08:00 的 11 月 24 日 00:00（含）到 11 月 29 日 00:00（不含）为日期窗口。检查所有本地引用及 reflog，未查远端。
- Git 选入 7 个提交记录。其中 `b472ed6ded` 是 11 月 26 日对 11 月 22 日版本 `082e9bd3e3` 的 amend，二者父提交相同。验收净差异以 `git diff 082e9bd3e3 4796b71dbb` 为准（10 个文件），不是把 11 月 22 日已完成的整笔监听器工作重新归到本轮。原始 amend 对照以 reflog 为定位手段，新 checkout 不保证保留 `082e9bd3e3`。
- amend 的实际增量是 Canvas 两条测试、二/三元组 `Lerp`、IDE 测试源目录配置及账本文字。Canvas 生产优化属于之后的 `858596d37e`。监听器实现与独立 harness 在 amend 前后无差异，复跑作为关联基线。
- `.idea/Hgpui.iml` 的确在 amend 中增加了现存的 `crates/gpui_refineable/tests` 源目录；它不是未提交改动，本轮不改动。IshiTone、独立组件库源码和旧 38 项剩余优化不进入验收范围。

| 提交 | 验收对象 |
|---|---|
| `b472ed6ded`（amend 增量） | 二/三元组 `Lerp`、Canvas 验证入口、IDE 测试目录、账本修订 |
| `858596d37e` / C07 | Canvas 样式按需分配，已有布局及绘制保留 |
| `6fb7e236e2` / C08 | `animated_uniform_list` 双缓冲、稳态分配、动画及跟踪键生命周期 |
| `7f75bcda29` / C09 | `uniform_list` 代表行测量复用、Auto 行为保留、同帧尺寸 |
| `5518043d00` / C10 | `animated_list` 行 ID 兼容和分配请求、实际行动画 |
| `835fe6502b` | 核心组件文档收尾、停止门与保护项 |
| `4796b71dbb` | `Deferred::ignore_mask()` 的默认行为、显式 opt-in、视口和局部裁剪、缓存重放 |

原始验收 HEAD 本机核心库：**477 通过 / 5 忽略 / 0 失败**，包括已有的 Deferred 两模式场景测试；该运行先于本轮新增测试和修复。Git 净差异通过 `git diff --check 082e9bd3e3 4796b71dbb`。`hgpui_refineable --tests` 实跑 **10 通过**（optional nested 7、ranges 3）。

## C08 验收发现：空闲前释放已删除行键

1. **发现**：两张跟踪表的桶可以复用，但若旧表到下一帧才清空，旧表仍持有行键 `K`。若非动画列表从 1 行变 0 行，不会请求后续动画帧，行键附带的所有权可能随空闲视图长期滞留。
2. **定位与红测**：使用真实 `ListState::advance`，行键用 `Rc<u64>`；先证明有行时跟踪表持有它，再绘制一个空帧，不启用离场动画。原实现实际失败：`the scratch map must not retain removed row keys`。同批新增的 Tuple Lerp、Auto/Infer、Deferred 缓存裁剪三项已通过。
3. **修复**：最后一次读取旧表后就在当帧清空条目，保留桶容量。清的是跟踪暂存表，不是离场动画有意持有的数据；当前帧的表、锚点、滑动、enter/leave 语义不变。
4. **转绿与留档**：空帧后弱引用不能升级，且没有依赖额外动画帧。10,000 键夹具暖机后 100 帧的稳态分配仍是 **0 次 / 0 B**，列表的插入、删除、锚点、滚动和离场等状态机或场景回归随核心全测通过。修复独立提交 `ee0a39ce33`。

## 成果复测

| 对象 | 本次结果 | 数据边界 |
|---|---|---|
| C07 Canvas | 无样式 `Canvas<()>` 内联 **40 B**，测得构造分配 **0 次 / 0 B**；有样式的 Canvas 布局为 24 × 16，蓝色背景进入 Scene | 内联体积不含单独分配；回调捕获也可能分配。历史同配置旧体积为 1384 B |
| C08 动画均匀列表 | 暖机后 **100 帧零分配**；上一帧表不再滞留删除键 | 保留两张表的桶，不将分配次数等同于总 live heap。历史同夹具旧实现为 400 次 / 156,400 B |
| C09 UniformList | **Auto 2 / Infer 2** 次回调每帧；改变行高后滚动高度正确 | 历史 Infer 旧实现为 3 次，现版少一次代表行构建和布局 |
| C10 动画列表行 ID | **100 次生成零分配**；保留旧格式和键哈希；实际 enter/leave 回归通过 | 每个列表实例仍创建一次命名空间，列表依旧非虚拟。历史旧行 ID 函数为 100 次 / 1,300 B |
| 监听器关联回归 | 4 监听器请求 2 次 / 88 B；高密度 action 16 项请求 3 次 / 896 B | debug 分配请求；同机 Release 对照见后文 |

表中的旧版本数值来自[此前实际红测记录](core-elements-performance.md)，不是本次重新编译旧版本；当前版数据为本次运行所得。

## 新强化的功能边界

- **元组插值**：直接覆盖二元组和嵌套三元组，混用 `Pixels`、整数、`f32` 和 `f64`，验证两个端点及中间值按成员顺序计算，不加 feature 也纳入核心全测。
- **UniformList**：同一实际窗口分别验证 Auto 和 Infer；行高从 20 改到 30 后，每帧仍恰好一次代表行测量、一次可见范围构建，100 行的滚动内容高度从 2000 更新到 3000，不是跨帧缓存旧尺寸。
- **Deferred**：保留默认继承圆角裁剪与 opt-in 两模式测试；新增一个弹层真实越过祖先与视口边缘的场景，检查视口裁剪和弹层内部圆角裁剪仍在。只通知根视图，断言子视图渲染次数不变，再检查重放后的 Scene，确保实际走了缓存重放；不是 GPU 像素或鼠标命中测试。

## 监听器关联基线：复跑 Release 独立夹具

Windows MSVC，Rust/Cargo 1.98.0。独立 harness 直接导入生产 `ListenerList`，opt-level 3，7 轮交替顺序中位数。以下为本次实测，非应用帧耗时或 RSS。

| 回调数 / clone | 旧 Vec → 紧凑表：分配请求数/字节 | 旧 Vec → 紧凑表：ns/节点 |
|---|---:|---:|
| 0 / 否 | 0/0 → 0/0 | 7.72 → 1.28 |
| 1 / 否 | 1/64 → 1/24 | 90.09 → 82.13 |
| 1 / 是 | 2/80 → 1/24 | 154.91 → 83.57 |
| 4 / 否 | 1/64 → 2/88 | 95.34 → 178.24 |
| 16 / 是 | 4/704 → 5/728 | 453.83 → 603.66 |

空与单回调收益和多回调 spill 权衡均复现；最后一行是泛型稀疏表夹具，不是已恢复 Vec 的 dense action 字段。复跑未更改生产实现。

## 本机验收边界

- 核心专项 **23 通过 / 1 忽略**（忽略项是手动监听器基准，独立 Release 夹具已实际运行）；修复并强化后的核心全库 **480 通过 / 5 忽略 / 0 失败**。
- 渲染契约 **43 通过**；Windows 原生库 **48 通过 / 1 忽略**（既有 GUI 字符映射测试）；独立 WGPU shader validation **1 通过**。
- Windows `wgpu` feature 检查、Windows 宿主的 workspace `--all-targets` 编译检查与补丁空白检查均通过。本轮的验收范围在本机通过。
- 本轮没有重开组件优化或审计余项。未启动/修改 IshiTone，未做整应用 Release 帧时、工作集或显存 A/B；没有在 macOS 编译或运行。上述内存、分配、计时是各自夹具的证据，不外推成整应用收益。

## 可复跑命令

从 Hgpui 仓库根执行。Windows/GPU 测试单线程，WGPU shader 独立进程；`CARGO_INCREMENTAL=0` 只对本次 shell 生效。查范围时使用 `git log --all --reflog --since-as-filter='2026-11-24T00:00:00+08:00' --until='2026-11-29T00:00:00+08:00'`，再比较 `082e9bd3e3` 与 amend 后提交。

```sh
export CARGO_INCREMENTAL=0
cargo test -p hgpui --lib --locked --offline -- --test-threads=1 --nocapture \
  core_component_ removed_tracking_keys_are_released tuples_interpolate_members \
  uniform_list_measures_once elements::deferred
cargo test -p hgpui --lib --locked --offline -- --test-threads=1
cargo test -p hgpui_refineable --tests --locked --offline
cargo test -p hgpui_render --lib --locked --offline
cargo test -p hgpui_windows --lib --locked --offline -- --test-threads=1
cargo test -p hgpui_wgpu --test shader_validation --locked --offline -- --test-threads=1
cargo check -p hgpui_windows --features wgpu --locked --offline
cargo check --workspace --all-targets --locked --offline
CARGO_TARGET_DIR=target/core-element-bench cargo run \
  --manifest-path tooling/core-element-bench/Cargo.toml --release --locked --offline
```
