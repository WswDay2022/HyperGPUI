# Grid 定位层叠收尾：按端点细化

## 现象与最小复现

基线：`27bf6b112d`。基础样式指定列起点后，另一层样式只指定行起点，列起点却被清为 Auto。同轴修改一个端点也会清掉另一个端点。

```rust
let mut base = Style::default().refined(div().col_start(2).style().clone());
base.refine(&div().row_start(4).style().clone());
// 应保留 column.start = Line(2)，修复前实际得到 Auto。
```

已运行的失败反馈命令：

```sh
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib grid_location_refinement --offline --quiet
CARGO_INCREMENTAL=0 cargo test -p hgpui --lib grid_location_refinement_minimal_row_column_cascade --offline --quiet
```

修复前：三个回归全部失败；两次样式操作的最小复现也失败，断言输出 `left: Auto / right: Line(2)`。测试不依赖系统输入、时钟或 GPU，实际执行耗时不足一秒（首次编译另计）。

## 定位与排除

1. `Styled` 方法原来在每份 `StyleRefinement` 中创建完整 `GridLocation::default()`，因此未指定的端点也成为 Auto。
2. `Style.grid_location` 原来没有标记递归细化；派生宏按普通 `Option<T>` 处理，直接把整个记录替换为新记录。
3. 最小断言在调用 Taffy 前已失败，因此不需要扩大到 Taffy 算法、DPI 或渲染器假设排查。
4. 没有添加调试桩或临时日志；直接通过红测和生成代码确认赋值路径。

## 修复

- 在 Refineable 中为现有 `std::ops::Range<T>` 增加端点细化，新增稀疏的 `RangeRefinement<T>`；保留 GridLocation 原有的 Range 字段类型，不要求迁移完整位置的 Range 字面量。
- `GridLocation.row/column` 标记递归细化；`Style.grid_location` 作为可选结构同样按递归细化处理。
- 普通、未标记递归细化的 `Option<T>` 仍使用旧的整体替换行为。
- `None` 表示不更新；`Some(GridPlacement::Auto)` 表示明确重置该端点。
- `grid_row/grid_column`、span/full-span 只指定相应轴的两个端点；`grid_area` 明确指定四个端点。
- 空的可选嵌套补丁保持 no-op：不创建位置记录，不序列化空字段，`is_some/is_empty`、Debug、superset/subtract 与此保持一致。
- `subtract` 保留框架既有的“左侧剩余”语义：移除已被右侧匹配的端点，留下左侧原值；它不是生成朝向右侧值的更新补丁。
- 可选嵌套值首次创建时使用结构自身的 Default，随后应用非空补丁；不是直接用每个字段的默认值替代结构默认值。

## 兼容性

所有现有 Styled 定位方法的名称和参数不变。`Style.grid_location` 仍是 `Option<GridLocation>`。

必要的低层类型迁移：

```rust
// 完整位置的 row/column 类型保持不变。
let row: std::ops::Range<GridPlacement> = GridPlacement::Line(1)..GridPlacement::Line(3);

// 直接给 StyleRefinement 赋完整位置时，明确转换成四端点补丁。
refinement.grid_location = Some(complete_location.into());

// 稀疏定位补丁：未指定不是 Auto。
refinement.grid_location_mut().row.start = Some(GridPlacement::Line(2));
```

旧完整 JSON（row/column 各有 start/end）仍可读取，完整补丁写回的形状也保持一致。新补丁允许省略未修改的轴和端点：

```json
{"grid_location":{"row":{"end":"Auto"}}}
```

该补丁只重置行终点，不影响列及行起点。需要重置全部定位时，可传入完整 `GridLocation::default().into()`，或分别显式设置四个 Auto。

## 验证范围

- 原始最小复现与三个失败回归已转绿。
- 扩展回归覆盖同轴端点、跨轴 shorthand、命名区域、显式 Auto、空补丁、Cascade、superset/subtract、旧完整 JSON 与新稀疏 JSON。
- 测试窗口通过真实 focus 样式合并、布局、场景输出验证：焦点变化只移动行位置，列坐标保持不变。
- Refineable 集成测试独立验证可选嵌套合并、普通 Option 整体替换、空补丁、结构 Default、refined/From/Cascade/superset/subtract。

2026-11-15 最终验证：核心 421 项（含 41 项 Grid 回归，其中 10 项为本轮定位收尾）、细化工具 10 项（7 optional + 3 range）、组件库 142 项、渲染契约 43 项测试全部通过。工作区全部 targets 检查通过；`grid_complete` 默认与 wgpu 两条编译路径通过。补丁空白检查通过。

本轮未启动外部示例窗口或修改 IshiTone；没有平台专属改动，macOS 未实机验证。
