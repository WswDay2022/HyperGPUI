# Taffy 布局能力透传

## 目标与范围

基线是本仓库固定依赖的 Taffy 0.12.2：把它已经实现、且能由当前 TaffyTree 使用的布局属性暴露给 Hgpui。没有新增 Grid 算法，不引入 subgrid、masonry 或 CSS 解析器。

Grid 布局最终仍由 TaffyTree 计算。着色器、平台后端和绘制顺序没有改变。

## 完整轨道接口

```rust
use hgpui::{div, px, Styled, GridTrackSizingFunction as Track};

let content = div().grid().grid_template_columns([
    Track::length(px(120.)),
    Track::flex(1.),
    Track::flex(2.),
]);
```

- `length(...)` 接收 px、rem、百分比。
- `fr(n)` 是 `minmax(auto, nfr)`，符合 Taffy 的普通 fr 语义。
- `flex(n)` 是 `minmax(0, nfr)`，符合旧 `grid_cols` 的零最小值语义。
- `auto()`、`min_content()`、`max_content()`、`fit_content(...)`、`minmax(min, max)` 提供其余已有轨道类型。
- `GridTemplateComponent::Repeat(GridTemplateRepetition::new(count, tracks))` 表达重复的多轨道组；count 支持 `Count(u16)`、`AutoFill`、`AutoFit`。
- `GridTemplateTracks::with_line_names` 表达外层轨道组之间的线名；重复内部的线名使用 `GridTemplateRepetition::with_line_names`。
- 线名数组按 Taffy 的 component 边界组织。转换时补齐缺少的空边界，重复组内部同样补齐，调用者不用为 unnamed 边界自行填空。

## 其他 Grid 属性

| 方法/类型 | 对应能力 |
|---|---|
| `grid_template_rows/columns` | 完整显式模板 |
| `grid_auto_rows/columns` | 循环应用的隐式轨道大小 |
| `grid_auto_flow(GridAutoFlow)` | Row/Column/RowDense/ColumnDense |
| `grid_template_areas` | 一基线坐标的命名矩形区域 |
| `grid_area(name)` | 将子项放入命名区域 |
| `grid_row/column(start..end)` | 完整行/列端点，支持 line/span 混合 |
| `GridPlacement::NamedLine(name, occurrence)` | 命名线的正/负第 n 次出现 |
| `GridPlacement::NamedSpan(name, count)` | 到指定命名线的跨度 |
| `align_items`、`self_align` | 单元格 block 轴对齐 |
| `justify_items`、`justify_self` | 单元格 inline 轴对齐 |
| `align_content`、`justify_content` | 整组轨道的空间分布 |
| `AlignItems/AlignContent::Safe*` | 溢出时回退到 start 的 positional 对齐 |

`justify_content` 不等于 `justify_items`；后者控制每个 Grid 单元格内子项的位置。

## 共享布局属性

- `layout_direction(LayoutDirection::Ltr/Rtl)`：布局方向，不冒充文字 shaping 或动画方向。
- `box_sizing(BoxSizing::BorderBox/ContentBox)`：现有默认仍是 BorderBox。
- `item_is_replaced(bool)`、`item_is_table(bool)`：Taffy 已有的 intrinsic/block sizing 标记。
- `float(FloatSide)`、`clear(ClearFloats)`：Taffy 已有的块布局浮动/清除规则。
- `block_text_align(BlockTextAlign)`：Taffy 的 legacy block child alignment；不改变已有 `text_align` 的文本绘制接口。
- `layout_border(Edges<DefiniteLength>)`：布局专用 absolute/percentage border metrics，不会绘制边框。也可显式使用 `LayoutBorder::Widths { top, right, bottom, left }`；`LayoutBorder::Paint` 恢复使用原有绘制边框的设备像素吸附宽度。

## 兼容性

旧便捷方法签名及语义保留：

| 方法 | 模板 |
|---|---|
| `grid_cols(n)` / `grid_rows(n)` | `repeat(n, minmax(0, 1fr))` |
| `*_min_content(n)` | `repeat(n, minmax(min-content, 1fr))` |
| `*_max_content(n)` | `repeat(n, minmax(0, max-content))` |

新旧方法写入同一个 `grid_cols/grid_rows` 字段，因此遵从原有调用顺序和 StyleRefinement 的层叠顺序；空显式模板是实际的清空操作，不会意外回退到旧模板。

明确的 Rust 类型迁移：

1. `Style/StyleRefinement.grid_cols/grid_rows` 从 `Option<GridTemplate>` 升级为 `Option<GridTemplateDefinition>`。直接赋值旧结构时改为 `Some(GridTemplate { ... }.into())`；旧便捷方法不用改。
2. `GridPlacement` 因新增字符串线名不再实现 `Copy`。数值构造方式不变，需要重复持有值的代码使用 `clone()`。
3. 对齐枚举新增 Safe* 变体；调用者的穷尽 match 需要覆盖它们。

旧 GridTemplate 的 JSON 对象形状仍可读取和写回，不要求迁移现有配置。新增类型保留 Serde 与 JsonSchema 能力。

## Grid 定位的层叠语义

定位按行起点、行终点、列起点、列终点独立细化。hover/focus 等补丁只更新明确指定的端点，不再以默认 Auto 清空其他端点。

- 完整 `Style.grid_location` 仍是 `Option<GridLocation>`；`GridLocation.row/column` 仍是 `std::ops::Range<GridPlacement>`。
- `StyleRefinement.grid_location` 使用 `Option<GridLocationRefinement>`；两个轴的补丁是 `RangeRefinement<GridPlacement>`。
- 补丁端点 `None` 表示不修改；`Some(GridPlacement::Auto)` 表示明确重置。
- 直接给 StyleRefinement 赋完整 GridLocation 时写 `Some(location.into())`；低层补丁端点赋值写 `Some(placement)`。
- 旧完整定位 JSON 仍可读取，完整补丁可保持原有写回形状。新稀疏 JSON 省略未修改的轴/端点，显式 Auto 不会被省略。
- `grid_row/grid_column`、span/full-span 更新对应轴的两个端点；`grid_area` 指定四端点。
- 普通、未标记递归细化的 Option 字段仍是整体替换，不改变其他样式的既有规则。

排查与回归记录见 `docs/grid-location-refinement-fix.md`。

## 单位与默认成本

绝对轨道长度先按当前 rem 大小转成逻辑 px，再应用现有设备像素吸附；百分比与 fr 不提前求值、不进行 DPI 缩放。所有新增选项默认保持原来的 Taffy 默认行为；未启用名称时不生成名称边界数组。

## Taffy 自身的边界

- auto-repeat 每轴至多一个，且需要该轴轨道满足 Taffy 的 fixed-component 规则；repeat 不嵌套。
- NamedSpan 的部分组合会按 Taffy 的规则回退；命名区域不是 CSS 字符串矩阵解析器。
- numeric line(0)、极大轨道数量、非法/非有限数值等沿用 Taffy 的表示与限制；本接口不是完整 CSS 输入校验器。
- subgrid、masonry、writing-mode、CSS order 输入、完整浏览器 CSS 属性词汇不在当前 Taffy 能力范围。
- calc 只有 opaque handle 表示；当前 TaffyTree resolver 恒返回 0，不暴露一个假可用的 calc 接口。
- 内置树的叶子测量回调只返回尺寸，不能直接提供真实文字 baseline；保留现有 Taffy baseline 行为。

## 示例与验证入口

```sh
cargo run -p hgpui --example grid_complete
cargo run -p hgpui --example grid_complete --features wgpu
cargo test -p hgpui --lib grid
cargo test -p hgpui --lib
cargo check --workspace --all-targets
```

回归覆盖 Styled → StyleRefinement → Style → Taffy 的调用路径，以及轨道转换、真实布局、层叠覆盖和序列化。另有测试窗口从 Render/Div 布局到 Scene quad 的接线验证。

2026-11-15 已执行（含定位层叠收尾）：核心 421 项、细化工具 10 项、组件库 142 项、渲染契约 43 项测试全部通过；Grid 专属回归共 41 项（包含在核心测试内）。`cargo check --workspace --all-targets --offline` 通过；`grid_complete` 示例在默认 Windows 后端与 `--features wgpu` 下均通过编译检查。新增文件已格式化，`git diff --check` 通过。

本轮没有启动示例窗口做视觉验收；macOS 不在本机实机验证范围内。
