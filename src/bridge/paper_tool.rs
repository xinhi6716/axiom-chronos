use super::Bridge;
use crate::{council, paper::members::PaperMembers};
use std::{marker::PhantomData, path::Path};

pub struct ProductionMembers<'a> {
    _lifetime: PhantomData<&'a ()>,
}

#[allow(unused_variables)]
impl<'a> ProductionMembers<'a> {
    pub fn new(bridge: &'a Bridge, setup: &'a council::Setup, store: &Path, cwd: &Path) -> Self {
        unimplemented!()
    }

    pub fn members(&self) -> PaperMembers<'_> {
        unimplemented!()
    }
}
