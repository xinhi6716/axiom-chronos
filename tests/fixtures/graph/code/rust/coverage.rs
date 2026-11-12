use crate::alpha::{self, beta as bee, nested::{Thing, *}};
use self::local::item;
use super::outer;
mod external;
pub mod inner {
    pub struct Boxed<T> { value: T }
    pub enum Choice { One }
    pub trait Work { fn execute(&self, input: crate::Input); }
    pub type Alias = crate::Input;
    pub const LIMIT: usize = 4;
    pub static GLOBAL: usize = 1;
    macro_rules! sample { () => {} }
    pub(crate) fn helper() { crate::api::run(); }
    impl<T> crate::model::Boxed<T> {
        pub fn method(&self, arg: crate::Input) {
            self.x.y.run();
            factory().dynamic();
            sample!();
        }
    }
}
fn scopes(param: fn(), input: Option<fn()>) {
    param();
    let local = make();
    local();
    let copy = local;
    { let nested = make(); nested(); }
    nested();
    let closure = |captured: fn()| captured();
    match input { Some(matched) => matched(), None => other() }
    if let Some(bound) = input { bound(); } else { bound(); }
    for iter in items { iter(); }
    unbound();
}