//! Hgpui-owned grid track definitions and their device-pixel-aware Taffy conversion.

use crate::{
    AbsoluteLength, DefiniteLength, GridTemplate, GridTemplateMinSize, Pixels, Relative, Rems,
    SharedString,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use taffy::style_helpers::TaffyFitContent;

/// A grid template, accepting both the legacy uniform template and explicit tracks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum GridTemplateDefinition {
    /// The original `repeat` and `min_size` representation.
    Legacy(GridTemplate),
    /// Individual tracks, repetitions, and optional names for their boundary lines.
    Tracks(GridTemplateTracks),
}

impl Default for GridTemplateDefinition {
    fn default() -> Self {
        Self::Tracks(GridTemplateTracks::default())
    }
}

impl From<GridTemplate> for GridTemplateDefinition {
    fn from(template: GridTemplate) -> Self {
        Self::Legacy(template)
    }
}

impl<T: Into<GridTemplateComponent>> From<Vec<T>> for GridTemplateDefinition {
    fn from(tracks: Vec<T>) -> Self {
        Self::Tracks(GridTemplateTracks::new(tracks))
    }
}

impl<T: Into<GridTemplateComponent>, const N: usize> From<[T; N]> for GridTemplateDefinition {
    fn from(tracks: [T; N]) -> Self {
        Self::from(Vec::from(tracks))
    }
}

impl From<GridTemplateTracks> for GridTemplateDefinition {
    fn from(tracks: GridTemplateTracks) -> Self {
        Self::Tracks(tracks)
    }
}

impl GridTemplateDefinition {
    pub(crate) fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> (Vec<taffy::GridTemplateComponent<String>>, Vec<Vec<String>>) {
        match self {
            Self::Legacy(template) => {
                let (min, max) = match template.min_size {
                    GridTemplateMinSize::Zero => (
                        taffy::MinTrackSizingFunction::length(0.0),
                        taffy::MaxTrackSizingFunction::fr(1.0),
                    ),
                    GridTemplateMinSize::MinContent => (
                        taffy::MinTrackSizingFunction::min_content(),
                        taffy::MaxTrackSizingFunction::fr(1.0),
                    ),
                    GridTemplateMinSize::MaxContent => (
                        taffy::MinTrackSizingFunction::length(0.0),
                        taffy::MaxTrackSizingFunction::max_content(),
                    ),
                };
                (
                    vec![taffy::GridTemplateComponent::Repeat(
                        taffy::GridTemplateRepetition {
                            count: taffy::RepetitionCount::Count(template.repeat),
                            tracks: vec![taffy::TrackSizingFunction { min, max }],
                            line_names: Vec::new(),
                        },
                    )],
                    Vec::new(),
                )
            }
            Self::Tracks(tracks) => tracks.to_taffy(rem_size, scale_factor),
        }
    }
}

/// An explicit template. Outer line names refer to component boundaries, not expanded tracks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridTemplateTracks {
    /// Individual tracks and repeat groups.
    pub tracks: Vec<GridTemplateComponent>,
    /// Names for each boundary, including the boundary after the last component.
    #[serde(default)]
    pub line_names: Vec<Vec<SharedString>>,
}

impl GridTemplateTracks {
    /// Creates an unnamed template from individual components.
    pub fn new<T: Into<GridTemplateComponent>>(tracks: impl IntoIterator<Item = T>) -> Self {
        Self {
            tracks: tracks.into_iter().map(Into::into).collect(),
            line_names: Vec::new(),
        }
    }

    /// Assigns names to component boundaries. Missing trailing boundaries are padded on conversion.
    pub fn with_line_names(mut self, line_names: impl Into<Vec<Vec<SharedString>>>) -> Self {
        self.line_names = line_names.into();
        self
    }

    pub(crate) fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> (Vec<taffy::GridTemplateComponent<String>>, Vec<Vec<String>>) {
        let named = has_line_names(&self.line_names)
            || self.tracks.iter().any(|component| match component {
                GridTemplateComponent::Single(_) => false,
                GridTemplateComponent::Repeat(repeat) => has_line_names(&repeat.line_names),
            });
        let tracks = self
            .tracks
            .iter()
            .map(|component| match component {
                GridTemplateComponent::Single(track) => {
                    taffy::GridTemplateComponent::Single(track.to_taffy(rem_size, scale_factor))
                }
                GridTemplateComponent::Repeat(repeat) => taffy::GridTemplateComponent::Repeat(
                    repeat.to_taffy_with_names(rem_size, scale_factor, named),
                ),
            })
            .collect();
        let line_names = to_taffy_line_names(&self.line_names, self.tracks.len() + 1, named);
        (tracks, line_names)
    }
}

/// One track or a repeated sequence of tracks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum GridTemplateComponent {
    /// A single track.
    Single(GridTrackSizingFunction),
    /// A fixed or automatic repetition.
    Repeat(GridTemplateRepetition),
}

/// Creates an automatic Grid track for fluent configuration.
///
/// ```
/// use hgpui::{div, px, repeat, track, Styled};
/// let grid = div().grid()
///     .grid_template_rows(repeat(3, [track().size(px(40.))]))
///     .grid_template_cols([track().size(px(120.)), track().fr(1.)]);
/// ```
pub fn track() -> GridTemplateComponent {
    GridTemplateComponent::Single(GridTrackSizingFunction::auto())
}

/// Repeats a sequence of tracks in an explicit Grid template.
///
/// Accepts integer counts, `GridRepetitionCount::AutoFill`, or `AutoFit`.
/// The result can be passed directly to a template setter or mixed with
/// `track()` values in an array. Nested repeats are not supported by Taffy
/// and are rejected rather than silently discarded.
pub fn repeat<T: Into<GridTrackSizingFunction>>(
    count: impl Into<GridRepetitionCount>,
    tracks: impl IntoIterator<Item = T>,
) -> GridTemplateComponent {
    GridTemplateComponent::Repeat(GridTemplateRepetition::new(
        count,
        tracks
            .into_iter()
            .map(Into::into)
            .collect::<Vec<GridTrackSizingFunction>>(),
    ))
}

impl GridTemplateComponent {
    /// Selects a fixed single track size in px, rem, or percent.
    pub fn size(self, length: impl Into<DefiniteLength>) -> Self {
        Self::Single(GridTrackSizingFunction::length(length))
    }

    /// Alias for `size`.
    pub fn length(self, length: impl Into<DefiniteLength>) -> Self {
        self.size(length)
    }

    /// Selects `minmax(auto, fraction fr)`.
    pub fn fr(self, fraction: f32) -> Self {
        Self::Single(GridTrackSizingFunction::fr(fraction))
    }

    /// Selects `minmax(0, fraction fr)`, matching the old equal-track helpers.
    pub fn flex(self, fraction: f32) -> Self {
        Self::Single(GridTrackSizingFunction::flex(fraction))
    }

    /// Selects an automatic single track.
    pub fn auto(self) -> Self {
        Self::Single(GridTrackSizingFunction::auto())
    }

    /// Selects a min-content single track.
    pub fn min_content(self) -> Self {
        Self::Single(GridTrackSizingFunction::min_content())
    }

    /// Selects a max-content single track.
    pub fn max_content(self) -> Self {
        Self::Single(GridTrackSizingFunction::max_content())
    }

    /// Selects a fit-content single track.
    pub fn fit_content(self, limit: impl Into<DefiniteLength>) -> Self {
        Self::Single(GridTrackSizingFunction::fit_content(limit))
    }

    /// Selects a single track with explicit bounds.
    /// Bounds accept px/rem/percent, sizing-function variants, or single tracks.
    /// For example: `track().minmax(px(120.), track().fr(1.))`.
    pub fn minmax(
        self,
        min: impl Into<GridMinTrackSizingFunction>,
        max: impl Into<GridMaxTrackSizingFunction>,
    ) -> Self {
        Self::Single(GridTrackSizingFunction::minmax(min.into(), max.into()))
    }

    /// Updates only the minimum of a single track, preserving its maximum.
    pub fn min(self, min: impl Into<GridMinTrackSizingFunction>) -> Self {
        let mut sizing: GridTrackSizingFunction = self.into();
        sizing.min = min.into();
        Self::Single(sizing)
    }

    /// Updates only the maximum of a single track, preserving its minimum.
    pub fn max(self, max: impl Into<GridMaxTrackSizingFunction>) -> Self {
        let mut sizing: GridTrackSizingFunction = self.into();
        sizing.max = max.into();
        Self::Single(sizing)
    }
}

impl From<GridTrackSizingFunction> for GridTemplateComponent {
    fn from(track: GridTrackSizingFunction) -> Self {
        Self::Single(track)
    }
}

impl From<GridTemplateRepetition> for GridTemplateComponent {
    fn from(repetition: GridTemplateRepetition) -> Self {
        Self::Repeat(repetition)
    }
}

impl From<GridTemplateComponent> for GridTrackSizingFunction {
    fn from(component: GridTemplateComponent) -> Self {
        match component {
            GridTemplateComponent::Single(track) => track,
            GridTemplateComponent::Repeat(_) => {
                panic!("expected a single Grid track; repeat is not valid here")
            }
        }
    }
}

impl From<GridTemplateComponent> for GridTemplateDefinition {
    fn from(component: GridTemplateComponent) -> Self {
        Self::from(vec![component])
    }
}

impl From<GridTrackSizingFunction> for GridTemplateDefinition {
    fn from(sizing: GridTrackSizingFunction) -> Self {
        Self::from(GridTemplateComponent::Single(sizing))
    }
}

/// A repeated sequence of tracks with optional names for its internal boundaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridTemplateRepetition {
    /// How many times to repeat the sequence.
    pub count: GridRepetitionCount,
    /// The track sequence to repeat.
    pub tracks: Vec<GridTrackSizingFunction>,
    /// Names for track boundaries, including the boundary after the last track.
    #[serde(default)]
    pub line_names: Vec<Vec<SharedString>>,
}

impl GridTemplateRepetition {
    /// Creates an unnamed repetition. Integer counts can be passed directly.
    pub fn new(
        count: impl Into<GridRepetitionCount>,
        tracks: impl Into<Vec<GridTrackSizingFunction>>,
    ) -> Self {
        Self {
            count: count.into(),
            tracks: tracks.into(),
            line_names: Vec::new(),
        }
    }

    /// Assigns names to repeated track boundaries.
    pub fn with_line_names(mut self, line_names: impl Into<Vec<Vec<SharedString>>>) -> Self {
        self.line_names = line_names.into();
        self
    }

    fn to_taffy_with_names(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
        named: bool,
    ) -> taffy::GridTemplateRepetition<String> {
        taffy::GridTemplateRepetition {
            count: self.count.into(),
            tracks: self
                .tracks
                .iter()
                .map(|track| track.to_taffy(rem_size, scale_factor))
                .collect(),
            // Even an unnamed repeat must advance the named resolver over its boundaries.
            line_names: to_taffy_line_names(&self.line_names, self.tracks.len() + 1, named),
        }
    }
}

/// The repetition count of a grid track group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum GridRepetitionCount {
    /// Repeat exactly this many times.
    Count(u16),
    /// Create enough repetitions to fill the available space.
    AutoFill,
    /// Like auto-fill, but collapse unused tracks.
    AutoFit,
}

impl From<u16> for GridRepetitionCount {
    fn from(count: u16) -> Self {
        Self::Count(count)
    }
}

impl From<i32> for GridRepetitionCount {
    fn from(count: i32) -> Self {
        Self::Count(u16::try_from(count).expect("Grid repeat count must be between 0 and 65535"))
    }
}

impl From<usize> for GridRepetitionCount {
    fn from(count: usize) -> Self {
        Self::Count(u16::try_from(count).expect("Grid repeat count must be between 0 and 65535"))
    }
}

impl From<GridRepetitionCount> for taffy::RepetitionCount {
    fn from(count: GridRepetitionCount) -> Self {
        match count {
            GridRepetitionCount::Count(count) => Self::Count(count),
            GridRepetitionCount::AutoFill => Self::AutoFill,
            GridRepetitionCount::AutoFit => Self::AutoFit,
        }
    }
}

/// The minimum and maximum sizing functions of a grid track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridTrackSizingFunction {
    /// The minimum track sizing function.
    pub min: GridMinTrackSizingFunction,
    /// The maximum track sizing function.
    pub max: GridMaxTrackSizingFunction,
}

impl GridTrackSizingFunction {
    /// A fixed pixel, rem, or percentage track size.
    pub fn length(length: impl Into<DefiniteLength>) -> Self {
        let length = length.into();
        Self::minmax(
            GridMinTrackSizingFunction::Length(length),
            GridMaxTrackSizingFunction::Length(length),
        )
    }

    /// A fractional track with an automatic minimum (`minmax(auto, fraction fr)`).
    pub fn fr(fraction: f32) -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::Auto,
            GridMaxTrackSizingFunction::Fraction(fraction),
        )
    }

    /// A fractional track with a zero minimum (`minmax(0, fraction fr)`).
    pub fn flex(fraction: f32) -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::Length(DefiniteLength::Absolute(AbsoluteLength::Pixels(
                Pixels(0.0),
            ))),
            GridMaxTrackSizingFunction::Fraction(fraction),
        )
    }

    /// An automatically sized track.
    pub fn auto() -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::Auto,
            GridMaxTrackSizingFunction::Auto,
        )
    }

    /// A track sized to its minimum content contribution.
    pub fn min_content() -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::MinContent,
            GridMaxTrackSizingFunction::MinContent,
        )
    }

    /// A track sized to its maximum content contribution.
    pub fn max_content() -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::MaxContent,
            GridMaxTrackSizingFunction::MaxContent,
        )
    }

    /// A track with an automatic minimum and a fit-content limit.
    pub fn fit_content(limit: impl Into<DefiniteLength>) -> Self {
        Self::minmax(
            GridMinTrackSizingFunction::Auto,
            GridMaxTrackSizingFunction::FitContent(limit.into()),
        )
    }

    /// A track with explicit minimum and maximum sizing functions.
    pub fn minmax(min: GridMinTrackSizingFunction, max: GridMaxTrackSizingFunction) -> Self {
        Self { min, max }
    }

    pub(crate) fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::TrackSizingFunction {
        taffy::TrackSizingFunction {
            min: self.min.to_taffy(rem_size, scale_factor),
            max: self.max.to_taffy(rem_size, scale_factor),
        }
    }
}

macro_rules! grid_bound_length_conversions {
    ($($ty:ty),+ $(,)?) => { $(
        impl From<$ty> for GridMinTrackSizingFunction {
            fn from(length: $ty) -> Self { Self::Length(length.into()) }
        }
        impl From<$ty> for GridMaxTrackSizingFunction {
            fn from(length: $ty) -> Self { Self::Length(length.into()) }
        }
    )+ };
}

grid_bound_length_conversions!(Pixels, Rems, Relative, AbsoluteLength, DefiniteLength);

impl From<GridTrackSizingFunction> for GridMinTrackSizingFunction {
    fn from(track: GridTrackSizingFunction) -> Self {
        track.min
    }
}
impl From<GridTrackSizingFunction> for GridMaxTrackSizingFunction {
    fn from(track: GridTrackSizingFunction) -> Self {
        track.max
    }
}
impl From<GridTemplateComponent> for GridMinTrackSizingFunction {
    fn from(track: GridTemplateComponent) -> Self {
        GridTrackSizingFunction::from(track).min
    }
}
impl From<GridTemplateComponent> for GridMaxTrackSizingFunction {
    fn from(track: GridTemplateComponent) -> Self {
        GridTrackSizingFunction::from(track).max
    }
}

/// The minimum sizing function of a grid track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum GridMinTrackSizingFunction {
    /// A pixel, rem, or percentage minimum.
    Length(DefiniteLength),
    /// An automatically computed minimum.
    Auto,
    /// The minimum content contribution.
    MinContent,
    /// The maximum content contribution.
    MaxContent,
}

impl GridMinTrackSizingFunction {
    pub(crate) fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::MinTrackSizingFunction {
        match self {
            Self::Length(length) => to_taffy_length(*length, rem_size, scale_factor).into(),
            Self::Auto => taffy::MinTrackSizingFunction::auto(),
            Self::MinContent => taffy::MinTrackSizingFunction::min_content(),
            Self::MaxContent => taffy::MinTrackSizingFunction::max_content(),
        }
    }
}

/// The maximum sizing function of a grid track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum GridMaxTrackSizingFunction {
    /// A pixel, rem, or percentage maximum.
    Length(DefiniteLength),
    /// An automatically computed maximum.
    Auto,
    /// The minimum content contribution.
    MinContent,
    /// The maximum content contribution.
    MaxContent,
    /// A share of the remaining space, in `fr` units.
    Fraction(f32),
    /// A fit-content limit in pixels, rems, or percent.
    FitContent(DefiniteLength),
}

impl GridMaxTrackSizingFunction {
    pub(crate) fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::MaxTrackSizingFunction {
        match self {
            Self::Length(length) => to_taffy_length(*length, rem_size, scale_factor).into(),
            Self::Auto => taffy::MaxTrackSizingFunction::auto(),
            Self::MinContent => taffy::MaxTrackSizingFunction::min_content(),
            Self::MaxContent => taffy::MaxTrackSizingFunction::max_content(),
            Self::Fraction(fraction) => taffy::MaxTrackSizingFunction::fr(*fraction),
            Self::FitContent(limit) => taffy::MaxTrackSizingFunction::fit_content(to_taffy_length(
                *limit,
                rem_size,
                scale_factor,
            )),
        }
    }
}

/// The direction and packing mode used to place grid items automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum GridAutoFlow {
    /// Fill rows, adding rows when needed.
    #[default]
    Row,
    /// Fill columns, adding columns when needed.
    Column,
    /// Fill rows and backfill earlier gaps.
    RowDense,
    /// Fill columns and backfill earlier gaps.
    ColumnDense,
}

impl From<GridAutoFlow> for taffy::GridAutoFlow {
    fn from(flow: GridAutoFlow) -> Self {
        match flow {
            GridAutoFlow::Row => Self::Row,
            GridAutoFlow::Column => Self::Column,
            GridAutoFlow::RowDense => Self::RowDense,
            GridAutoFlow::ColumnDense => Self::ColumnDense,
        }
    }
}

/// A named rectangular grid area, using Taffy's one-based, exclusive end-line coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridTemplateArea {
    /// The name by which items can refer to this area.
    pub name: SharedString,
    /// The first row line, starting at one.
    pub row_start: u16,
    /// The exclusive ending row line.
    pub row_end: u16,
    /// The first column line, starting at one.
    pub column_start: u16,
    /// The exclusive ending column line.
    pub column_end: u16,
}

impl GridTemplateArea {
    pub(crate) fn to_taffy(&self) -> taffy::GridTemplateArea<String> {
        taffy::GridTemplateArea {
            name: self.name.to_string(),
            row_start: self.row_start,
            row_end: self.row_end,
            column_start: self.column_start,
            column_end: self.column_end,
        }
    }
}

fn to_taffy_length(
    length: DefiniteLength,
    rem_size: Pixels,
    scale_factor: f32,
) -> taffy::LengthPercentage {
    match length {
        DefiniteLength::Absolute(length) => taffy::LengthPercentage::length(
            crate::util::round_to_device_pixel(length.to_pixels(rem_size).0, scale_factor),
        ),
        DefiniteLength::Relative(relative) => taffy::LengthPercentage::percent(relative.as_f32()),
    }
}

fn has_line_names(line_names: &[Vec<SharedString>]) -> bool {
    line_names.iter().any(|names| !names.is_empty())
}

fn to_taffy_line_names(
    line_names: &[Vec<SharedString>],
    boundary_count: usize,
    named: bool,
) -> Vec<Vec<String>> {
    let mut names: Vec<Vec<String>> = line_names
        .iter()
        .map(|names| names.iter().map(ToString::to_string).collect())
        .collect();
    if named && names.len() < boundary_count {
        names.resize_with(boundary_count, Vec::new);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::{
        GridAutoFlow, GridMaxTrackSizingFunction, GridMinTrackSizingFunction, GridRepetitionCount,
        GridTemplateArea, GridTemplateComponent, GridTemplateDefinition, GridTemplateRepetition,
        GridTemplateTracks, GridTrackSizingFunction,
    };
    use crate::{GridTemplate, GridTemplateMinSize, Pixels, px, relative, rems};

    fn sizing(
        min: taffy::MinTrackSizingFunction,
        max: taffy::MaxTrackSizingFunction,
    ) -> taffy::TrackSizingFunction {
        taffy::TrackSizingFunction { min, max }
    }

    #[test]
    fn legacy_templates_preserve_repeat_and_complete_sizing() {
        for (min_size, min, max) in [
            (
                GridTemplateMinSize::Zero,
                taffy::MinTrackSizingFunction::length(0.0),
                taffy::MaxTrackSizingFunction::fr(1.0),
            ),
            (
                GridTemplateMinSize::MinContent,
                taffy::MinTrackSizingFunction::min_content(),
                taffy::MaxTrackSizingFunction::fr(1.0),
            ),
            (
                GridTemplateMinSize::MaxContent,
                taffy::MinTrackSizingFunction::length(0.0),
                taffy::MaxTrackSizingFunction::max_content(),
            ),
        ] {
            let definition = GridTemplateDefinition::from(GridTemplate {
                repeat: 3,
                min_size,
            });
            let (tracks, line_names) = definition.to_taffy(Pixels(16.0), 2.0);
            assert!(line_names.is_empty());
            assert_eq!(
                tracks,
                vec![taffy::GridTemplateComponent::Repeat(
                    taffy::GridTemplateRepetition {
                        count: taffy::RepetitionCount::Count(3),
                        tracks: vec![sizing(min, max)],
                        line_names: Vec::new(),
                    }
                )]
            );
        }
    }

    #[test]
    fn mixed_tracks_convert_units_without_scaling_percent_or_fractions() {
        let definition = GridTemplateDefinition::from([
            GridTrackSizingFunction::length(px(10.25)),
            GridTrackSizingFunction::length(rems(1.25)),
            GridTrackSizingFunction::length(relative(0.25)),
            GridTrackSizingFunction::fr(3.0),
            GridTrackSizingFunction::flex(2.0),
            GridTrackSizingFunction::minmax(
                GridMinTrackSizingFunction::Length(relative(0.1).into()),
                GridMaxTrackSizingFunction::FitContent(rems(1.25).into()),
            ),
            GridTrackSizingFunction::fit_content(relative(0.75)),
            GridTrackSizingFunction::fit_content(px(10.25)),
        ]);
        let (tracks, line_names) = definition.to_taffy(Pixels(16.0), 2.0);
        assert!(line_names.is_empty());
        let expected = [
            sizing(
                taffy::MinTrackSizingFunction::length(20.0),
                taffy::MaxTrackSizingFunction::length(20.0),
            ),
            sizing(
                taffy::MinTrackSizingFunction::length(40.0),
                taffy::MaxTrackSizingFunction::length(40.0),
            ),
            sizing(
                taffy::MinTrackSizingFunction::percent(0.25),
                taffy::MaxTrackSizingFunction::percent(0.25),
            ),
            sizing(
                taffy::MinTrackSizingFunction::auto(),
                taffy::MaxTrackSizingFunction::fr(3.0),
            ),
            sizing(
                taffy::MinTrackSizingFunction::length(0.0),
                taffy::MaxTrackSizingFunction::fr(2.0),
            ),
            sizing(
                taffy::MinTrackSizingFunction::percent(0.1),
                taffy::MaxTrackSizingFunction::fit_content_px(40.0),
            ),
            sizing(
                taffy::MinTrackSizingFunction::auto(),
                taffy::MaxTrackSizingFunction::fit_content_percent(0.75),
            ),
            sizing(
                taffy::MinTrackSizingFunction::auto(),
                taffy::MaxTrackSizingFunction::fit_content_px(20.0),
            ),
        ];
        assert_eq!(
            tracks,
            expected
                .into_iter()
                .map(taffy::GridTemplateComponent::Single)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn intrinsic_track_constructors_preserve_both_bounds() {
        for (track, expected) in [
            (
                GridTrackSizingFunction::auto(),
                sizing(
                    taffy::MinTrackSizingFunction::auto(),
                    taffy::MaxTrackSizingFunction::auto(),
                ),
            ),
            (
                GridTrackSizingFunction::min_content(),
                sizing(
                    taffy::MinTrackSizingFunction::min_content(),
                    taffy::MaxTrackSizingFunction::min_content(),
                ),
            ),
            (
                GridTrackSizingFunction::max_content(),
                sizing(
                    taffy::MinTrackSizingFunction::max_content(),
                    taffy::MaxTrackSizingFunction::max_content(),
                ),
            ),
        ] {
            assert_eq!(track.to_taffy(Pixels(16.0), 1.5), expected);
        }
    }

    #[test]
    fn fixed_and_auto_repetitions_preserve_count_and_track_sequence() {
        for (count, expected_count) in [
            (
                GridRepetitionCount::Count(4),
                taffy::RepetitionCount::Count(4),
            ),
            (
                GridRepetitionCount::AutoFill,
                taffy::RepetitionCount::AutoFill,
            ),
            (
                GridRepetitionCount::AutoFit,
                taffy::RepetitionCount::AutoFit,
            ),
        ] {
            let definition = GridTemplateDefinition::from([GridTemplateComponent::Repeat(
                GridTemplateRepetition::new(
                    count,
                    [
                        GridTrackSizingFunction::length(px(10.0)),
                        GridTrackSizingFunction::length(relative(0.2)),
                    ],
                ),
            )]);
            let (tracks, line_names) = definition.to_taffy(Pixels(16.0), 2.0);
            assert!(line_names.is_empty());
            let taffy::GridTemplateComponent::Repeat(repeat) = &tracks[0] else {
                panic!("expected a repeat group");
            };
            assert_eq!(repeat.count, expected_count);
            assert_eq!(
                repeat.tracks,
                vec![
                    sizing(
                        taffy::MinTrackSizingFunction::length(20.0),
                        taffy::MaxTrackSizingFunction::length(20.0)
                    ),
                    sizing(
                        taffy::MinTrackSizingFunction::percent(0.2),
                        taffy::MaxTrackSizingFunction::percent(0.2)
                    ),
                ]
            );
            assert!(repeat.line_names.is_empty());
        }
    }

    #[test]
    fn outer_names_pad_unnamed_repeat_boundaries() {
        let definition = GridTemplateDefinition::from(
            GridTemplateTracks::new([
                GridTemplateComponent::Repeat(GridTemplateRepetition::new(
                    2_u16,
                    [
                        GridTrackSizingFunction::length(px(10.0)),
                        GridTrackSizingFunction::length(px(20.0)),
                    ],
                )),
                GridTrackSizingFunction::auto().into(),
            ])
            .with_line_names(vec![vec!["start".into()]]),
        );
        let (tracks, names) = definition.to_taffy(Pixels(16.0), 1.0);
        assert_eq!(names, vec![vec!["start".to_string()], vec![], vec![]]);
        let taffy::GridTemplateComponent::Repeat(repeat) = &tracks[0] else {
            panic!("expected a repeat group");
        };
        assert_eq!(repeat.line_names, vec![Vec::<String>::new(); 3]);
    }

    #[test]
    fn inner_names_pad_outer_boundaries_and_other_repeats() {
        for count in [
            GridRepetitionCount::Count(2),
            GridRepetitionCount::AutoFill,
            GridRepetitionCount::AutoFit,
        ] {
            let definition = GridTemplateDefinition::from([
                GridTemplateComponent::Repeat(GridTemplateRepetition::new(
                    3_u16,
                    [GridTrackSizingFunction::length(px(5.0))],
                )),
                GridTemplateComponent::Repeat(
                    GridTemplateRepetition::new(count, [GridTrackSizingFunction::length(px(10.0))])
                        .with_line_names(vec![vec!["repeat-start".into()]]),
                ),
            ]);
            let (tracks, names) = definition.to_taffy(Pixels(16.0), 1.0);
            assert_eq!(names, vec![Vec::<String>::new(); 3]);
            let taffy::GridTemplateComponent::Repeat(first) = &tracks[0] else {
                panic!("expected an unnamed repeat group");
            };
            let taffy::GridTemplateComponent::Repeat(second) = &tracks[1] else {
                panic!("expected a named repeat group");
            };
            assert_eq!(first.line_names, vec![Vec::<String>::new(); 2]);
            assert_eq!(
                second.line_names,
                vec![vec!["repeat-start".to_string()], vec![]]
            );
        }
    }

    #[test]
    fn complete_boundary_names_are_preserved() {
        let definition = GridTemplateDefinition::from(
            GridTemplateTracks::new([GridTemplateComponent::Repeat(
                GridTemplateRepetition::new(2_u16, [GridTrackSizingFunction::length(px(10.0))])
                    .with_line_names(vec![vec!["inner-start".into()], vec!["inner-end".into()]]),
            )])
            .with_line_names(vec![
                vec!["start".into()],
                vec!["end".into(), "alias".into()],
            ]),
        );
        let (tracks, names) = definition.to_taffy(Pixels(16.0), 1.0);
        assert_eq!(
            names,
            vec![
                vec!["start".to_string()],
                vec!["end".to_string(), "alias".to_string()]
            ]
        );
        let taffy::GridTemplateComponent::Repeat(repeat) = &tracks[0] else {
            panic!("expected a repeat group");
        };
        assert_eq!(
            repeat.line_names,
            vec![
                vec!["inner-start".to_string()],
                vec!["inner-end".to_string()]
            ]
        );
    }

    #[test]
    fn defaults_and_track_collection_conversions_agree() {
        assert_eq!(
            GridTemplateDefinition::default(),
            GridTemplateDefinition::Tracks(GridTemplateTracks::default())
        );
        assert_eq!(
            GridTemplateDefinition::default().to_taffy(Pixels(16.0), 2.0),
            (Vec::new(), Vec::new())
        );
        let components: [GridTemplateComponent; 1] = [GridTrackSizingFunction::fr(1.0).into()];
        assert_eq!(
            GridTemplateDefinition::from(components.clone()),
            GridTemplateDefinition::from(Vec::from(components.clone()))
        );
        assert_eq!(
            GridTemplateDefinition::from(components.clone()),
            GridTemplateDefinition::from(GridTemplateTracks::new(components))
        );
    }

    #[test]
    fn serde_accepts_and_reemits_legacy_templates_unchanged() {
        for min_size in [
            GridTemplateMinSize::Zero,
            GridTemplateMinSize::MinContent,
            GridTemplateMinSize::MaxContent,
        ] {
            let legacy = GridTemplate {
                repeat: 3,
                min_size,
            };
            let json = serde_json::to_value(legacy).unwrap();
            let definition: GridTemplateDefinition = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(definition, GridTemplateDefinition::Legacy(legacy));
            assert_eq!(serde_json::to_value(&definition).unwrap(), json);
        }
    }

    #[test]
    fn serde_explicit_tracks_round_trip_and_allow_omitted_line_names() {
        let definition = GridTemplateDefinition::from(
            GridTemplateTracks::new([
                GridTrackSizingFunction::length(rems(1.5)).into(),
                GridTrackSizingFunction::fit_content(relative(0.5)).into(),
                GridTemplateComponent::Repeat(GridTemplateRepetition::new(
                    GridRepetitionCount::AutoFit,
                    [GridTrackSizingFunction::flex(2.0)],
                )),
            ])
            .with_line_names(vec![vec!["start".into()]]),
        );
        let json = serde_json::to_value(&definition).unwrap();
        assert_eq!(
            serde_json::from_value::<GridTemplateDefinition>(json).unwrap(),
            definition
        );
        let empty: GridTemplateDefinition = serde_json::from_str(r#"{"tracks":[]}"#).unwrap();
        assert_eq!(empty, GridTemplateDefinition::default());
        let repeat: GridTemplateRepetition =
            serde_json::from_str(r#"{"count":{"Count":2},"tracks":[]}"#).unwrap();
        assert_eq!(repeat, GridTemplateRepetition::new(2_u16, Vec::new()));
    }

    #[test]
    fn area_conversion_preserves_one_based_exclusive_end_lines() {
        let area = GridTemplateArea {
            name: "content".into(),
            row_start: 1,
            row_end: 3,
            column_start: 2,
            column_end: 5,
        };
        assert_eq!(
            area.to_taffy(),
            taffy::GridTemplateArea {
                name: "content".to_string(),
                row_start: 1,
                row_end: 3,
                column_start: 2,
                column_end: 5,
            }
        );
    }

    #[test]
    fn auto_flow_conversion_preserves_all_modes() {
        assert_eq!(GridAutoFlow::default(), GridAutoFlow::Row);
        for (flow, expected) in [
            (GridAutoFlow::Row, taffy::GridAutoFlow::Row),
            (GridAutoFlow::Column, taffy::GridAutoFlow::Column),
            (GridAutoFlow::RowDense, taffy::GridAutoFlow::RowDense),
            (GridAutoFlow::ColumnDense, taffy::GridAutoFlow::ColumnDense),
        ] {
            assert_eq!(taffy::GridAutoFlow::from(flow), expected);
        }
    }
}
