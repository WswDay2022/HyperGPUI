# 交叉审查后续：渲染、输入与无障碍一致性

## WGPU 变换剔除

见[渲染巡查记录](render-pipeline-resource-audit.md#wgpu--连续可见批次借用原数据)。生产批次选择器的回归覆盖移入、移出和连续可见项仍借用原切片；默认单位矩阵不映射四角。范围是 WGPU CPU 剔除，三端共享 shader 无改动。

## Deferred 继承裁剪

120px 窗口内，100px 宽矩形祖先下的浮层从 x=80 跨到 x=140。旧版仅恢复不能表示纯矩形裁剪的原始链，绘制使用 120px 的视口边界；预绘制也漏恢复遮罩，x=90 和 x=110 都能点击。正确值是默认边界 100px，只接受 x=90；显式 `.ignore_mask()` 才跳过祖先裁剪，仍保留视口和元素内部裁剪。

现在记录有效遮罩及未折叠的原始遮罩，以同一份上下文恢复预绘制与绘制。显式遮罩在原链中只记录一次，避免圆角/fade 重复；没有祖先遮罩时不建链，预绘制不复制 snapshot，绘制借用它。重放继续复用已记录的图元和命中框。未更改全局命中形状规则或嵌套 Deferred 的优先级语义。

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib --locked --offline elements::deferred -- --test-threads=1
```
