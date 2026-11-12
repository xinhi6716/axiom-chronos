mod a { pub fn measure() {} }
mod b { pub fn measure() {} }
use a::*;
use b::*;
fn run() { measure(); }
