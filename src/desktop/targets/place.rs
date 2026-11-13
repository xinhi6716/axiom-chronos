use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place { Secondary, Offscreen }
impl Place {
 pub fn key(self) -> &'static str { match self { Self::Secondary => "secondary", Self::Offscreen => "offscreen" } }
 pub fn from_key(value: &str) -> Option<Self> { match value { "secondary" => Some(Self::Secondary), "offscreen" => Some(Self::Offscreen), _ => None } }
}

pub struct DisplayInfo { pub id: u32, pub bounds: Bounds<Pixels>, pub primary: bool, pub refresh_hz: Option<f32> }
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen { Display { id: u32, origin: Point<Pixels> }, Offscreen }

pub fn choose(place: Place, displays: &[DisplayInfo]) -> Chosen {
 if place == Place::Secondary {
  if let Some(display) = displays.iter().find(|display| !display.primary) {
   return Chosen::Display { id: display.id, origin: display.bounds.origin + point(px(40.), px(40.)) };
  }
 }
 Chosen::Offscreen
}
pub fn window_bounds(chosen: &Chosen, size: Size<Pixels>) -> Bounds<Pixels> {
 let origin = match chosen { Chosen::Display { origin, .. } => *origin, Chosen::Offscreen => point(px(-3000.), px(-3000.)) };
 Bounds { origin, size }
}
pub fn refresh_hz_of(displays: &[DisplayInfo], window_display: u32) -> Option<f32> {
 displays.iter().find(|display| display.id == window_display).and_then(|display| display.refresh_hz)
}

pub struct Installed(pub Chosen);
impl Global for Installed {}

pub(crate) fn display_id(display: &dyn PlatformDisplay) -> u32 { u64::from(display.id()) as u32 }
pub(crate) fn display_infos(cx: &App) -> Vec<DisplayInfo> {
 let primary = cx.primary_display().map(|display| display.id());
 cx.displays().iter().map(|display| DisplayInfo {
  id: display_id(display.as_ref()), bounds: display.bounds(),
  primary: primary.map_or(true, |id| id == display.id()),
  refresh_hz: display.refresh_interval().and_then(|interval| {
   let seconds = interval.as_secs_f32();
   (seconds > 0.).then(|| 1. / seconds).filter(|rate| rate.is_finite())
  }),
 }).collect()
}
pub fn install(place: Place, cx: &mut App) { cx.set_global(Installed(choose(place, &display_infos(cx)))); }
pub fn installed(cx: &App) -> Option<Chosen> { cx.try_global::<Installed>().map(|installed| installed.0.clone()) }
pub fn bounds(size: Size<Pixels>, cx: &App) -> Option<Bounds<Pixels>> { installed(cx).map(|chosen| window_bounds(&chosen, size)) }
pub fn focus(cx: &App) -> bool { installed(cx).is_none() }

#[cfg(test)]
mod tests {
 use super::*;
 use std::prelude::v1::test;

 fn displays() -> Vec<DisplayInfo> {
  vec![
   DisplayInfo { id: 1, bounds: Bounds { origin: point(px(0.), px(0.)), size: size(px(2560.), px(1440.)) }, primary: true, refresh_hz: Some(170.) },
   DisplayInfo { id: 2, bounds: Bounds { origin: point(px(-1920.), px(0.)), size: size(px(1920.), px(1080.)) }, primary: false, refresh_hz: Some(60.) },
  ]
 }
 #[test]
 fn chooses_secondary_or_offscreen() {
  let displays = displays();
  let chosen = choose(Place::Secondary, &displays);
  match chosen {
   Chosen::Display { id, origin } => {
    assert_eq!(id, 2);
    assert!(!displays.iter().find(|d| d.id == id).unwrap().primary);
    let b = displays[1].bounds;
    assert_eq!(origin, point(px(-1880.), px(40.)));
    assert!(origin.x >= b.origin.x && origin.x < b.origin.x + b.size.width);
    assert!(origin.y >= b.origin.y && origin.y < b.origin.y + b.size.height);
   },
   _ => panic!("secondary was not chosen"),
  }
  assert!(matches!(choose(Place::Secondary, &displays[..1]), Chosen::Offscreen));
  assert!(matches!(choose(Place::Offscreen, &displays), Chosen::Offscreen));
  assert_eq!(window_bounds(&Chosen::Offscreen, size(px(10.), px(20.))).origin, point(px(-3000.), px(-3000.)));
 }
 #[test]
 fn refresh_uses_the_window_display() {
  let displays = displays();
  assert_eq!(refresh_hz_of(&displays, 2), Some(60.));
  assert_eq!(refresh_hz_of(&displays, 1), Some(170.));
  assert_eq!(refresh_hz_of(&displays, 99), None);
 }
}
