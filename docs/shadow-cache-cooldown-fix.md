# G09 — 阴影缓存峰值后主动回落

## 范围与基线

- 基线 main：`bcdceb7b4e`；工作分支：`fix/shadow-cache-cooldown`。
- 用户于 2026-11-22 限定：只完成 G09；随后停止其他优化，先验证前四阶段的正确性与实际收益。
- 不 push/合 main/删除分支，不纳入用户 `.idea/Hgpui.iml` 修改，不改 IshiTone 未提交工作。

## 发现 → 最小反馈

现有 64 项目标是软目标，不能硬删当前同时活跃的阴影。测试通过真实稳定性门及逐帧烘焙预算建立 80 个活跃 key，再让可见集合缩到一个已命中 key，完全不产生新 insert。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib shadow_cache_ --offline -- --nocapture --test-threads=1
```

实际修改前失败：

- CPU：`peak=80 retained=80 bytes=983040->983040`，断言 `retained 80 > 64`。
- Window 直接命中：`atlas 80->80, images 80->80`。
- Window cached paint 重放：`atlas 80->80, images 80->80`。
- 三项均在缓存无法回落的断言处失败，而非仅依赖 RSS 观察。

夹具调整：TestWindow 固定 2x DPI，每块像素是 CPU 单元夹具的四倍，10 帧不足以尊重真实预算烘焙全部 80 项；预热上限调整到 32 帧后，两个 Window 测试才到达上述正确失败点。这不是产品故障。首次在仓库外用 manifest-path 启动的构建未加载仓库 .cargo 配置，已停止，不当作回归结果；之后全部命令在仓库根运行。

## 定位与决策

1. 原 insert 才尝试驱逐，且一次最多移除一个条目；纯 hit/空闲绘制不会进入驱逐路径。
2. 不能仅依赖 lookup 更新帧龄：cached-view 直接重放 Scene 的 AtlasTile，不重新查 shadow key。
3. 不能把帧龄当作 GPU fence：DX UpdateSubresource/draw 与 WGPU Queue::write_texture 有提交顺序；Metal replace_region 是 CPU 写，必须等先前 GPU 读取真正完成后才复用纹素。
4. 帧边界回落还必须考虑最后一次内容变化后 UI 不再绘制：仅依靠测试强制连续 draw 会掩盖空闲边界。该边界单独红绿验证，不能为了清理维持永久动画循环。

## 实现

- 原 64 软目标、每块 512² 像素上限、每帧 256² 烘焙预算、稳定性门、解析 fallback 保持。
- insert 只插入；统一在 draw 开始、begin_frame 之后 trim，不在 paint 中提前驱逐随后可能被重放的图块。
- 缓存记录成功上传的 `(texture_id, tile_id)`；trim 用上一份仍可重放的 Scene 更新使用帧龄，保护当前/前一帧及 cached replay。
- 只在超目标时构造 live set/扫描/排序，按 LRU 移除安全候选；没有候选时允许当前活跃集合超过软目标，不丢阴影功能。
- Window 将 evicted image key 经 PlatformAtlas::retire 批量撤销，然后释放 CPU image owner；已有外部 Arc lease 保持有效。
- 只在超目标且当前 Scene 已不引用部分旧条目时安排有限的 grace 清理帧；callback 只设置窗口 dirty，不强制失效 cached-view。全活跃超目标集合不为软目标自发动画，收敛后停止请求帧。
- DX/WGPU 通过共同的默认 retire → remove 路径；相关 GPU 测试验证真实 atlas 页、热 key 及 WGPU pending upload 的释放。
- Metal 与 renderer 使用同一 CommandQueue；逻辑 key 先撤销，编码实际 GPU event marker，完成回调才 deallocate tile/释放空页。回调持 Arc 状态，涵盖窗口关闭；生产路径不 wait_until_completed。
- Metal 原 remove 的释放算法保留为共享 helper，不改变其他调用方的 remove 行为；没有更改 shader/Scene ABI。

## 空闲边界的追加红测

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib shadow_cache_window_idle_cooldown_requests_bounded_frames --offline -- --nocapture --test-threads=1
```

首轮仅帧边界 trim 的修复下，实际仍失败：`atlas 80->80, images 80->80`。测试只在可见集合变化时 draw 一次，之后必须由生产代码请求下一帧；没有无限强制 draw。补上有限 cooldown callback 后重新验证，并断言收敛后及全活跃峰值不请求额外帧。

## G09 修复提交前验证（2026-11-22）

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib shadow_cache_ --offline -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui_windows --lib shadow_retirement_ --offline -- --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p hgpui_wgpu --lib shadow_retirement_ --offline -- --nocapture --test-threads=1
```

- 专项 6 passed：无新 insert、当前/前一帧保护、LRU 与外部 Arc lease、Window atlas 撤销、cached replay、空闲有限清理帧。
- 核心全量 **450 passed / 2 ignored**；ignored 仍为原有两项手动基准。
- DX 原生 retirement **1 passed**；WGPU 原生 retirement **1 passed**，未 skip。
- CPU：80→64，983040→786432 字节；实际释放 **196608 字节**（该受控夹具的 20%）。
- Window：直接命中、cached replay、最后一次 draw 后空闲三条路径均 atlas 80→64、CPU image 80→64；热点 tile ID 不变，关闭后所有 image weak 无法升级。
- 原生 atlas：三个 1024² BGRA 页（引擎拥有的纹理面积字节 12582912）→一个热页（4194304），撤销两个空页、保留热 tile；WGPU pending upload 也从三个降到一个。
- `git diff --check` 与七个 Rust 文件的 rustfmt 语法解析通过（emit stdout、skip_children，不全仓格式化）。

这些证据是实际 CPU 图像所有权/缓冲释放与引擎 atlas 页撤销，不当作进程 RSS、驱动实际驻留显存或整体 UI/FPS 提升；已有命令/图形上下文也可能延后释放 GPU 原生引用。

## 验证边界与停止门

- Metal 新增真实队列阻塞/完成、同 key 重载、owner drop 测试；源码、队列一致性、API 签名及 Rust 语法静态核对完成。本机没有 Mac，不声称已编译或运行。
- 此修复与本记录单独本地提交；本文件不循环引用自身提交 hash，以 Git 日志和后续总验证记录为准。
- G09 修复闭环完成后转入前四阶段验证（含工作区/相关平台和复测收益），不推进 G02/T02、图片预算或 GPU pass 优化。
