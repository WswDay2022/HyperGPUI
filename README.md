# HyperGPUI

本项目是 [gpui-ce](https://github.com/gpui-ce/gpui-ce)(Zed [GPUI](https://gpui.rs) 的社区 fork)的独立 fork,用于:

- **学习** gpui 的架构(元素树、渲染后端、窗口系统)
- **扩展** — 在框架上持续添加自己需要的功能

> 远端: `git remote -v` 可见 `origin` 指向 gpui-ce 上游,`fork` 指向 `https://github.com/WswDay2022/HyperGPUI.git`。
>
> 版本传承: **Zed GPUI** → **gpui-ce**(社区 fork)→ **本 fork(HyperGPUI)**。

---

## 相对上游的本地改动

以下提交为本 fork 相对 `origin/main`(gpui-ce)的独有内容,按提交时间倒序:

### 1. `e3007b6be9` — BorderlessWindow 无边框窗口组件

将 `examples/learn/borderless_resizeable_window.rs` 中的无边框窗口模式封装为可复用组件,供应用直接使用:

- **文件**: `crates/gpui/src/elements/borderless_window.rs`(由 `elements/mod.rs` 导出)
- **功能**:
  - 透明全尺寸根节点 + 8 个不可见的边/角 resize 命中区(边 6px、角 12px)
  - 左键按下边/角 → `window.start_window_resize(edge)` → 走**系统原生 resize 循环**(Windows 上为 `WM_SYSCOMMAND SC_SIZE`,自带实时预览与 snap layouts)
  - 每个命中区显示对应的系统 resize 光标
- **API**:
  ```rust
  // 打开窗口
  cx.open_window(BorderlessWindow::options(Bounds::centered(None, size(px(480.), px(360.)), cx)), ...);

  // 渲染内容
  BorderlessWindow::new()
      .inset(px(12.))          // 内容四周留白(阴影边距)
      .edge_size(px(6.))       // 边命中区厚度
      .corner_size(px(12.))    // 角命中区大小
      .child(div().size_full().bg(rgb(0xFFFFFF)).shadow_lg())
  ```
- **已知坑**: `set_client_inset` 在 Windows 后端(`gpui_windows`)是 **no-op**,视觉阴影边距完全靠 `inset` 的内容 padding 实现;组件仍会调用它,以便在 Wayland/X11 上生效。
- **前提**: 窗口 `is_resizable` 必须为 `true`(`WindowOptions::default()` 即默认 `true`)。
- **配套示例**: `examples/learn/borderless_resizeable_window.rs` 演示完整用法(含 `.window_control_area(WindowControlArea::Drag)` 拖拽移动区域)。

### 2. `c63cc55c29` — 完整 transform 支持(修复上游 #93)

为 `div` 元素补全 CSS 风格 transform(translate / rotate / scale 等),并在三个渲染后端落地:

- **核心**: 新增 `crates/gpui/src/css_transform.rs`(约 480 行),接入 `div` / `styled` / `style` / `window` / `scene` / `inspector`
- **三后端着色器**:
  - Windows:`gpui_windows/src/shaders.hlsl` + `directx_renderer.rs`(D3D11)
  - macOS:`gpui_macos/src/shaders.metal`(Metal)
  - WGPU:`gpui_wgpu/src/shaders.wgsl` + `wgpu_renderer.rs`
- **附带修复(Windows)** — HLSL quad 渲染失效根因: `QuadVertexOutput` 声明了 `SV_ClipDistance` 而 `QuadFragmentInput` 没有,导致 FXC 按声明顺序分配寄存器时 TEXCOORD 错位、像素着色器不运行、矩形不绘制;已在两处对齐声明修复。
- **附带工具(Windows)**: D3D11 debug layer(`D3D11_CREATE_DEVICE_DEBUG` + `ID3D11InfoQueue` 消息转储)调试工具链,以及 `shader_layout_tests.rs` 着色器签名布局测试。
- **学习示例**(`crates/gpui/examples/learn/`): `blur_test`、`borderless_resizeable_window`、`button_feedback`、`counter`、`css_transform`。

---

## 构建

```sh
# Windows(Git Bash)
export PATH="/d/.cargo/bin:$PATH"
cargo build -p gpui-ce -j 1        # 注意 -j 1:本机内存有限,并行编译易 OOM
```

- workspace 默认成员为 `./crates/gpui/`(即 `gpui-ce` crate)
- 本 fork 的测试宏 `gpui::test` 导出 `test` 宏,平台 crate(`gpui_windows` 等)测试模块中禁止 `use super::*` 以避免冲突

## 来源与致谢

- **Zed GPUI**: https://gpui.rs / https://github.com/zed-industries/zed — 原始框架,GPU 加速的 Rust UI
- **gpui-ce**: https://github.com/gpui-ce/gpui-ce — 社区 fork,本 fork 的上游
- **本 fork 远端**: https://github.com/WswDay2022/HyperGPUI
- 无边框可调整窗口的原始模式参考 Zed 上游与 gpui-ce 的 `examples/learn/borderless_resizeable_window.rs`
- **Claude (Anthropic)**: 协助实现完整 transform 支持与 HLSL quad 渲染修复、封装 BorderlessWindow 组件,并编写本文档
