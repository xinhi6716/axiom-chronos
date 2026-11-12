mod a { pub fn compute() {} }
mod b { pub fn compute() {} }
use a::*;
use b::*;
fn run() { compute(); }
