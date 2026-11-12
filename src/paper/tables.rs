use crate::{
    base::Res,
    paper::{
        evidence::DataTable,
        figures::fonts::TextMeasure,
        model::{RenderedTable, TableSpec},
    },
};
pub fn booktabs(
    _spec: &TableSpec,
    _data: &DataTable,
    _text_width_mm: f64,
    _m: &dyn TextMeasure,
    _family: &str,
) -> Res<RenderedTable> {
    unimplemented!()
}
