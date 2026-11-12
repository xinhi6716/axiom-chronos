use super::{fonts::TextMeasure, layout::FigureLayout, presets::Preset};
use crate::{
    base::Res,
    paper::{evidence::DataTable, model::FigureSpec},
};
pub fn layout(
    _spec: &FigureSpec,
    _data: &DataTable,
    _preset: &Preset,
    _m: &dyn TextMeasure,
) -> Res<FigureLayout> {
    unimplemented!()
}
