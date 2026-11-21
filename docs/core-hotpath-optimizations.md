# 核心二次开销优化记录（R01 / G01 / R02）

## 基线与范围

- 基线 main：`d7aed1aa7b`；本轮分支 `fix/core-quadratic-hotpaths`。
- 按 R01 → G01 → R02 顺序验证及独立本地提交，不推送。
- 用户 Release 实机观察启动约 15 MB 内存；这是用户观察值，不据此宣称本轮每项单独带来的下降。
- 用户已有 `.idea/Hgpui.iml` 修改保留，不纳入提交。Mac 无实体环境，保持源码层面核对，不安装环境。

## R01 — FocusMap 最终释放驱动清理

### 修改前反馈

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib focus_cleanup_no_drop_effect_batch_does_not_scan_live_handles --offline --quiet
```

256 个始终存活的 focus handle + 64 个 effect，实际扫描 16640 个条目，即 (64+1)×256；应为 0。测试实际失败。

### 修复

- FocusHandle 仅最后引用 Drop 将 FocusId 放入所属 FocusMap 的释放队列。
- 没有释放信号时仅原子读取，不取得 focus slots/队列锁、不克隆 Arc 或扫描 table。
- 非空队列按 ID 批量删除，窗口更新/blur 在 write lock 释放后执行；再 drain 重入产生的释放。
- 队列仅持有 FocusId，不持 focus handle，不形成新的所有权环；弱升级不能复活 ref_count=0 的槽。
- 普通 read/write 接口和 FocusId generation 语义保留，测试验证非最终 Drop、有限 final drops、后台释放、窗口 blur。

### 夹具测量（debug，不是生产 UI 帧时）

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib focus_cleanup_effect_batch_benchmark --offline -- --ignored --nocapture --test-threads=1
```

每组 20 批次，时间包含测试夹具、effects 和 App update，不把以下倍率当成整应用性能倍率；首次编译不计入表。

| F handles | P effects | 原扫描数 | 修复后扫描数 | 原 elapsed ms | 修复后 elapsed ms |
|---:|---:|---:|---:|---:|---:|
|256|64|332800|0|13.154|0.647|
|256|256|1315840|0|69.931|2.506|
|1024|64|1331200|0|57.492|0.789|
|1024|256|5263360|0|215.862|2.174|
|4096|64|5324800|0|208.007|0.496|
|4096|256|21053440|0|861.367|1.983|

5 项功能回归通过，benchmark 作为显式 ignored 测量，不在普通测试中自动执行。提交：`4c8d5da9b1`。

## G01 — 空间树平衡与精确端点

原确定性反馈：2048 个 (2i,0,1,1) 不相交矩形，height=188（limit23），total_insert_path=191301（limit47104）。不使用计时阈值，实际 red。

实现保持 MAX_CHILDREN=12，真正把 overflow 13 个孩子分裂成6/7，并向父传播，所有叶子等深，非根内部节点占用至少6。结构插入高度受对数界限制，普通空间查询的最坏复杂度仍可能 O(n)，不宣称所有查询对数。

独立参考与结构测试暴露出 f32 union 的非结合舍入：metadata 精确重折叠可相差1 ULP，并且由 origin+size 重建端点可能向内舍入，造成真实少覆盖。不能只放宽断言。

必要修正：Node 额外缓存精确 max_corner；内部用子节点端点极值，不反复 subtract/add 重建；空间剪枝使用缓存端点，bounds.size只用于启发式。新增 (-4096,0,1,1) 与 (0,0,2^-12,1) 的确定性细条反例，验证查询/重复插入 order、相接边缘及保守覆盖。旧算法注释保留便于恢复。

7 项空间树回归通过。相同 2048 项输入，height 从188降至5，累计 insert_path 从191301降至7557；这是结构操作统计，不当成实际帧时倍率。固定seed参考ordering、严格边界及细条反例均通过。提交：`423f917d53`。

## R02 — 局部 cached-view 依赖捕获

新增最小反馈：frame 前缀已访问 entity A，cached 子 scope 再次读 A，应仍把 A 放入自己的依赖集合。旧全局 set difference 会遗漏它，同时 clone/difference 全局前缀有二次开销。

修改前最小回归已实际失败，读取过的共享实体不在 capture 输出内。

实现使用同一 RefCell 中的 global set 与 scope stack：record 只写 global 和 top scope，结束时向 parent scope 合并，不复制全局前缀。缓存重放同样记入 scope；frame clear/窗口注册只处理 global。实际 entity 释放从 global 和活动 scopes 同时删 ID。catch_unwind 后先关闭 scope 再 resume_unwind，保持嵌套与被捕获异常后的栈恢复。

7 项回归通过，包含共享/嵌套/重放、frame clear、释放、insert/update/as_mut 与 panic 恢复。释放夹具先 flush EntityCreated（它暂时持强 handle），再验证真正最后 Drop，未误把未释放对象当泄漏。提交：`6987fcbb64`。

前缀夹具固定每次局部读取1实体，共200次capture：

|prefix|原捕获依赖数|新捕获依赖数|原elapsed ms|新elapsed ms|
|---:|---:|---:|---:|---:|
|256|0|200|7.395|0.172|
|1024|0|200|27.837|0.183|
|4096|0|200|110.891|0.279|

以上为 debug 夹具，不是整应用FPS倍率；同时修复了依赖集合的错误遗漏。实际全局prefix不再参与每次capture复制/difference。

## 最终验证

- 核心：444 passed / 2 ignored（两个本轮手动基准，已单独运行并记录）。
- 组件库：142 passed。
- Windows：46 passed / 1 ignored（既有字符映射表 GUI 测试）。
- 渲染契约：43 passed。
- WGPU shader_validation：1 passed。
- 工作区全部 targets 编译检查通过；Windows wgpu feature 检查通过。
- 各项红测实际观察修改前失败，修复后通过；benchmark 数据仅用于对应夹具，不作整体 UI 倍率承诺。
- Mac 保留代码层面核对边界，没有安装/启动 Mac 环境；没有修改用户的 IDE 文件或 IshiTone 未提交工作。

这轮仅完成第 3 组 R01/G01/R02，未开始第 4、5 组预算和 GPU pass 优化。
