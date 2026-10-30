//! Shared typed rendering and shader contracts.

pub mod abi;
pub mod artifacts;
mod instances;
pub mod shaders;
mod target_stack;

pub use instances::InstanceRange;
pub use target_stack::TargetStack;

// `blur` 与 `path_types` 暂未接入：
// - 上游 `blur.rs` 依赖上游 `BlurUniforms` 的字段布局，本地模糊 uniform 仍携带 mask chain；
// - 上游 `path_types::PathRasterizationVertex` 的大小依赖上游 `Background` 布局，本地为 144B。
// 两者都需在 shader/ABI 对齐阶段一起迁移。
