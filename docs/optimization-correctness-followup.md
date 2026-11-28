# 交叉审查后续：渲染、输入与无障碍一致性

## WGPU 变换剔除

见[渲染巡查记录](render-pipeline-resource-audit.md#wgpu--连续可见批次借用原数据)。生产批次选择器的回归覆盖移入、移出和连续可见项仍借用原切片；默认单位矩阵不映射四角。范围是 WGPU CPU 剔除，三端共享 shader 无改动。

## Deferred 继承裁剪

120px 窗口内，100px 宽矩形祖先下的浮层从 x=80 跨到 x=140。旧版仅恢复不能表示纯矩形裁剪的原始链，绘制使用 120px 的视口边界；预绘制也漏恢复遮罩，x=90 和 x=110 都能点击。正确值是默认边界 100px，只接受 x=90；显式 `.ignore_mask()` 才跳过祖先裁剪，仍保留视口和元素内部裁剪。

现在记录有效遮罩及未折叠的原始遮罩，以同一份上下文恢复预绘制与绘制。显式遮罩在原链中只记录一次，避免圆角/fade 重复；没有祖先遮罩时不建链，预绘制不复制 snapshot，绘制借用它。重放继续复用已记录的图元和命中框。未更改全局命中形状规则或嵌套 Deferred 的优先级语义。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --locked --offline elements::deferred -- --test-threads=1
```

## 缓存子树的无障碍节点和动作

开启无障碍后，先完整绘制一个带 Click/Focus 的按钮。只通知父视图重绘时，旧实现清空了本帧 AccessKit 节点和动作映射，却只重放子视图的图元与命中框。红测：按钮原有节点 ID 在下一帧消失。

在既有的无障碍帧分支使用窗口的 refresh 路径：只在无障碍启用时重建子树，缓存使用的布局样式不变。不加入热路径缓存查找分支、不保留第二份树或回调、不新增刷新帧。代价是启用时每个已绘制帧重建缓存子树；这是正确性回退，不是无障碍高速重放优化。关闭后正常缓存命中不变。回归验证父视图重绘后节点 ID、Focus 和 Click 都有效，并验证启用前和关闭后的渲染计数确实保持缓存命中。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --locked --offline cached_view_keeps_accessible_actions_after_parent_redraw -- --test-threads=1
```

测试使用真实 Window 帧构建、AccessKit 树和动作分发；未通过系统屏幕阅读器做人工验收。
