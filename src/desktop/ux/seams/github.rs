use std::{collections::VecDeque, sync::Mutex, time::Duration};
#[derive(Clone, Debug, PartialEq)]
pub enum GhOutcome {
    Exit {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Missing,
    TimedOut,
}
pub trait GhRunner: Send + Sync + 'static {
    fn run(&self, args: &[String], timeout: Duration) -> GhOutcome;
}
#[derive(Default)]
pub struct FakeGh {
    state: Mutex<(VecDeque<GhOutcome>, Vec<Vec<String>>)>,
}
impl FakeGh {
    pub fn new(script: Vec<GhOutcome>) -> Self {
        Self {
            state: Mutex::new((script.into(), vec![])),
        }
    }
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.state.lock().unwrap().1.clone()
    }
}
impl GhRunner for FakeGh {
    fn run(&self, args: &[String], _: Duration) -> GhOutcome {
        let mut s = self.state.lock().unwrap();
        s.1.push(args.to_vec());
        s.0.pop_front().unwrap_or(GhOutcome::Missing)
    }
}
