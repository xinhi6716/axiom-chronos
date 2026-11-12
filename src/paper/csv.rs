use crate::base::Res;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn parse(_text: &str, _delimiter: u8) -> Res<CsvTable> {
    unimplemented!()
}
pub fn split_unit(_header: &str) -> (String, Option<String>) {
    unimplemented!()
}
