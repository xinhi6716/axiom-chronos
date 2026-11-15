mod a { pub fn render() {} }
mod b { pub fn render() {} }
use a::*;
use b::*;
fn run() { render(); }
