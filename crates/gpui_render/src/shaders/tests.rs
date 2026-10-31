use super::*;

#[test]
fn validates_subpixel_shader() {
    let generated = subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap();
    wgsl_rs::validate_wgsl_source(&format!("enable dual_source_blending;\n{generated}")).unwrap();
}

#[test]
fn build_generated_shaders_match_rust_sources() {
    assert_eq!(
        base::WGSL_SOURCE.wgsl_source().unwrap(),
        crate::artifacts::BASE_WGSL
    );
    assert_eq!(
        format!(
            "enable dual_source_blending;\n{}",
            subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap()
        ),
        crate::artifacts::SUBPIXEL_WGSL
    );
}

#[test]
fn shader_interface_matches_generated_sources() {
    let standard_source = base::WGSL_SOURCE.wgsl_source().unwrap();
    let subpixel_source = subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap();
    for pipeline in interface::ALL {
        let source = if pipeline.label == interface::SUBPIXEL_SPRITES.label {
            &subpixel_source
        } else {
            &standard_source
        };
        assert!(
            source.contains(&format!("fn {}(", pipeline.vertex_entry)),
            "missing vertex entry point {}",
            pipeline.vertex_entry,
        );
        assert!(
            source.contains(&format!("fn {}(", pipeline.fragment_entry)),
            "missing fragment entry point {}",
            pipeline.fragment_entry,
        );
    }

    assert_eq!(common::GLOBALS.group, interface::GLOBAL_BIND_GROUP);
    assert_eq!(common::GLOBALS.binding, interface::GLOBAL_UNIFORMS_BINDING);
    assert_eq!(
        common::FONT_RASTERIZATION.group,
        interface::GLOBAL_BIND_GROUP
    );
    assert_eq!(
        common::FONT_RASTERIZATION.binding,
        interface::FONT_RASTERIZATION_BINDING
    );
    assert_eq!(quad::QUADS.group(), interface::DATA_BIND_GROUP);
    assert_eq!(quad::QUADS.binding(), interface::DATA_BUFFER_BINDING);
}

fn vertex_output_shape(module: &naga::Module, entry_name: &str) -> (usize, u32) {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == entry_name)
        .unwrap_or_else(|| panic!("missing shader entry point {entry_name}"));
    let result = entry
        .function
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("shader entry point {entry_name} has no output"));
    let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("shader entry point {entry_name} must return a struct");
    };

    members
        .iter()
        .filter(|member| matches!(member.binding, Some(naga::Binding::Location { .. })))
        .fold((0, 0), |(locations, components), member| {
            let member_components = match module.types[member.ty].inner {
                naga::TypeInner::Scalar(_) => 1,
                naga::TypeInner::Vector { size, .. } => u32::from(size),
                ref ty => panic!("unsupported stage output type {ty:?} in {entry_name}"),
            };
            (locations + 1, components + member_components)
        })
}

#[test]
fn ordinary_vertex_interfaces_stay_within_compact_budgets() {
    let module = naga::front::wgsl::parse_str(&base::WGSL_SOURCE.wgsl_source().unwrap()).unwrap();
    for (entry, maximum_shape) in [
        // The local CSS-transform patch adds `local_position` + `scene_position` + the flat
        // `aa` factor, so every transformed pipeline gets three extra locations.
        ("vertex_smoothed_quad", (16, 51)),
        ("vertex_smoothed_shadow", (10, 37)),
        ("vertex_smoothed_polychrome_sprite", (10, 25)),
        ("blur_composite_vertex", (2, 6)),
    ] {
        let actual_shape = vertex_output_shape(&module, entry);
        assert!(
            actual_shape.0 <= maximum_shape.0 && actual_shape.1 <= maximum_shape.1,
            "{entry} uses {actual_shape:?}, exceeding {maximum_shape:?} (locations, components)"
        );
    }
}

#[test]
fn scene_instance_storage_sizes_are_bounded() {
    // Budgets include the local transform matrices and the upstream dash/smoothing fields.
    for (name, actual, maximum) in [
        ("Quad", std::mem::size_of::<hgpui::Quad>(), 304),
        ("Shadow", std::mem::size_of::<hgpui::Shadow>(), 208),
        (
            "PolychromeSprite",
            std::mem::size_of::<hgpui::PolychromeSprite>(),
            160,
        ),
    ] {
        assert!(
            actual <= maximum,
            "{name} is {actual} bytes, exceeding its {maximum}-byte storage budget"
        );
    }
}

#[test]
fn shader_buffer_layouts_match_host_layouts() {
    let modules = [
        crate::artifacts::BASE_WGSL,
        crate::artifacts::SUBPIXEL_WGSL,
    ]
    .map(|source| naga::front::wgsl::parse_str(source).unwrap());

    for layouts in [interface::SCENE_BUFFER_LAYOUTS] {
        for layout in layouts {
            let ty = modules
                .iter()
                .find_map(|module| {
                    module
                        .types
                        .iter()
                        .find_map(|(_, ty)| (ty.name.as_deref() == Some(layout.name)).then_some(ty))
                })
                .unwrap_or_else(|| panic!("missing shader ABI type {}", layout.name));
            let naga::TypeInner::Struct { members, span } = &ty.inner else {
                panic!("shader ABI type {} must be a struct", layout.name);
            };

            assert_eq!(
                (*span) as usize, layout.size,
                "shader ABI size: {}",
                layout.name
            );
            assert_eq!(
                members.len(),
                layout.fields.len(),
                "shader ABI fields: {}",
                layout.name
            );
            for (member, (name, offset)) in members.iter().zip(layout.fields) {
                assert_eq!(
                    member.name.as_deref(),
                    Some(*name),
                    "shader ABI field: {}",
                    layout.name
                );
                assert_eq!(
                    member.offset as usize, *offset,
                    "shader ABI offset: {}.{name}",
                    layout.name
                );
            }
        }
    }
}

#[test]
fn shader_discriminants_match_scene_types() {
    // These are the values also used by `hgpui::color::BackgroundTag` and the
    // scene enums; keeping them literal avoids exposing those private tags.
    assert_eq!(common::ShaderBool::Disabled as u32, 0);
    assert_eq!(common::ShaderBool::Enabled as u32, 1);
    assert_eq!(common::BackgroundTag::Solid as u32, 0);
    assert_eq!(common::BackgroundTag::LinearGradient as u32, 1);
    assert_eq!(common::BackgroundTag::PatternSlash as u32, 2);
    assert_eq!(common::BackgroundTag::Checkerboard as u32, 3);
    assert_eq!(common::BackgroundTag::RadialGradient as u32, 4);
    assert_eq!(common::BackgroundTag::RadialGradientCircle as u32, 5);
    assert_eq!(common::ColorSpace::Srgb as u32, 0);
    assert_eq!(common::ColorSpace::Oklab as u32, 1);
    assert_eq!(common::BorderStyle::Solid as u32, 0);
    assert_eq!(common::BorderStyle::Dashed as u32, 1);
}

#[test]
fn srgb_gradients_interpolate_in_native_srgb_space() {
    use common::*;
    use wgsl_rs::std::*;

    let black = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 1.0,
    };
    let white = Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: 1.0,
    };
    let background = Background {
        tag: BackgroundTag::LinearGradient,
        color_space: ColorSpace::Srgb,
        solid: black,
        gradient_angle_or_pattern_height: 90.0,
        colors: [
            LinearColorStop {
                color: black,
                percentage: 0.0,
            },
            LinearColorStop {
                color: white,
                percentage: 1.0,
            },
        ],
        radial_center_y: 0.0,
        radial_size: 0,
        conic_center_x: 0.0,
    };
    let bounds = Bounds {
        origin: vec2f(0.0, 0.0),
        size: vec2f(100.0, 100.0),
    };
    let paint = Paint::new(background, bounds);
    let prepared = prepare_paint(paint);
    // Upstream gpui lerps sRGB stops directly (CSS behaviour): the midpoint of black -> white is
    // ~0.5, not the gamma-shifted ~0.21 that an encode/decode round-trip would produce.
    let middle = paint_color(paint, vec2f(50.0, 50.0), prepared);
    assert!(
        (middle.x - 0.5).abs() < 0.02,
        "sRGB gradient midpoint should be ~0.5, got {}",
        middle.x
    );
    let start = paint_color(paint, vec2f(0.0, 50.0), prepared);
    assert!(start.x < 0.02, "first stop should stay black, got {}", start.x);
    let end = paint_color(paint, vec2f(100.0, 50.0), prepared);
    assert!(end.x > 0.98, "last stop should stay white, got {}", end.x);
}

#[test]
fn conic_gradient_sweeps_clockwise_from_twelve_oclock() {
    use common::*;
    use wgsl_rs::std::*;

    let black = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 1.0,
    };
    let white = Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: 1.0,
    };
    let background = Background {
        tag: BackgroundTag::ConicGradient,
        color_space: ColorSpace::Srgb,
        solid: black,
        gradient_angle_or_pattern_height: 0.0,
        colors: [
            LinearColorStop {
                color: black,
                percentage: 0.0,
            },
            LinearColorStop {
                color: white,
                percentage: 1.0,
            },
        ],
        radial_center_y: 0.5,
        radial_size: 0,
        conic_center_x: 0.5,
    };
    let bounds = Bounds {
        origin: vec2f(0.0, 0.0),
        size: vec2f(100.0, 100.0),
    };
    let paint = Paint::new(background, bounds);
    let prepared = prepare_paint(paint);

    let sample = |x: f32, y: f32| paint_color(paint, vec2f(x, y), prepared).x;
    assert!(sample(50.0, 0.0) < 0.03, "12 o'clock is the start stop");
    assert!(
        (sample(100.0, 50.0) - 0.25).abs() < 0.05,
        "3 o'clock is a quarter turn clockwise"
    );
    assert!(
        (sample(50.0, 100.0) - 0.5).abs() < 0.05,
        "6 o'clock is half a turn clockwise"
    );
    assert!(
        (sample(0.0, 50.0) - 0.75).abs() < 0.05,
        "9 o'clock is three quarters clockwise"
    );

    // `from 90deg` rotates the start to 3 o'clock, so 12 o'clock lands at 0.75.
    let rotated = Background {
        gradient_angle_or_pattern_height: 90.0,
        ..background
    };
    let rotated_paint = Paint::new(rotated, bounds);
    let rotated_prepared = prepare_paint(rotated_paint);
    let top = paint_color(rotated_paint, vec2f(50.0, 0.0), rotated_prepared).x;
    assert!(
        (top - 0.75).abs() < 0.05,
        "a 90 degree start moves the sweep by a quarter turn, got {top}"
    );
}

#[test]
fn linear_gradients_preserve_native_dithering() {
    use common::*;
    use wgsl_rs::std::*;

    let color = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.5,
        a: 0.5,
    };
    let background = Background {
        tag: BackgroundTag::LinearGradient,
        color_space: ColorSpace::Srgb,
        solid: color,
        gradient_angle_or_pattern_height: 90.0,
        colors: [
            LinearColorStop {
                color,
                percentage: 0.0,
            },
            LinearColorStop {
                color,
                percentage: 1.0,
            },
        ],
        radial_center_y: 0.0,
        radial_size: 0,
        conic_center_x: 0.0,
    };
    let bounds = Bounds {
        origin: vec2f(0.0, 0.0),
        size: vec2f(100.0, 100.0),
    };
    let paint = Paint::new(background, bounds);
    let prepared = prepare_paint(paint);
    let first = paint_color(paint, vec2f(10.0, 10.0), prepared);
    let second = paint_color(paint, vec2f(11.0, 10.0), prepared);

    assert_ne!(first.w, second.w);
}
