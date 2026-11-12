use crate::base::Res;
use std::sync::Mutex;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnSettings {
    pub extreme: bool,
    pub revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FastCost {
    pub multiplier: f32,
}
pub trait TurnPolicy: Send + Sync + 'static {
    fn current(&self) -> TurnSettings;
    fn set_extreme(&self, on: bool) -> TurnSettings;
    fn try_set_extreme(&self, on: bool) -> Res<TurnSettings> {
        Ok(self.set_extreme(on))
    }
    fn begin_turn(&self) -> TurnSettings;
    fn fast_cost(&self) -> Option<FastCost>;
}
#[derive(Default)]
struct State {
    settings: TurnSettings,
    turns: Vec<TurnSettings>,
    cost: Option<FastCost>,
}
#[derive(Default)]
pub struct FakeTurnPolicy {
    state: Mutex<State>,
}
impl FakeTurnPolicy {
    pub fn new(cost: Option<FastCost>) -> Self {
        Self {
            state: Mutex::new(State {
                cost,
                ..Default::default()
            }),
        }
    }
    pub fn turns(&self) -> Vec<TurnSettings> {
        self.state.lock().unwrap().turns.clone()
    }
}
impl TurnPolicy for FakeTurnPolicy {
    fn current(&self) -> TurnSettings {
        self.state.lock().unwrap().settings
    }
    fn set_extreme(&self, on: bool) -> TurnSettings {
        let mut s = self.state.lock().unwrap();
        if s.settings.extreme != on {
            s.settings.extreme = on;
            s.settings.revision += 1;
        }
        s.settings
    }
    fn begin_turn(&self) -> TurnSettings {
        let mut s = self.state.lock().unwrap();
        let t = s.settings;
        s.turns.push(t);
        t
    }
    fn fast_cost(&self) -> Option<FastCost> {
        self.state.lock().unwrap().cost
    }
}
