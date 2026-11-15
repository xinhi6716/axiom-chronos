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

/// The top-left corner, in logical pixels on both axes, of an offscreen window: outside every display.
pub(crate) const OFFSCREEN_ORIGIN: f32 = -3000.;
/// How far inside a secondary display's top-left corner, on both axes, a placed window opens.
const DISPLAY_INSET: f32 = 40.;
pub(crate) fn offscreen_origin() -> Point<Pixels> { point(px(OFFSCREEN_ORIGIN), px(OFFSCREEN_ORIGIN)) }

pub fn choose(place: Place, displays: &[DisplayInfo]) -> Chosen {
 if place == Place::Secondary {
  if let Some(display) = displays.iter().find(|display| !display.primary) {
   return Chosen::Display { id: display.id, origin: display.bounds.origin + point(px(DISPLAY_INSET), px(DISPLAY_INSET)) };
  }
 }
 Chosen::Offscreen
}
pub fn window_bounds(chosen: &Chosen, size: Size<Pixels>) -> Bounds<Pixels> {
 let origin = match chosen { Chosen::Display { origin, .. } => *origin, Chosen::Offscreen => offscreen_origin() };
 Bounds { origin, size }
}
pub(crate) struct WindowInputs { pub bounds: Bounds<Pixels>, pub display_id: Option<u32>, pub show: bool, pub focus: bool }
pub(crate) fn window_inputs(chosen: &Chosen, size: Size<Pixels>) -> WindowInputs {
 WindowInputs {
  bounds: window_bounds(chosen, size),
  display_id: match chosen { Chosen::Display { id, .. } => Some(*id), Chosen::Offscreen => None },
  show: false, focus: false,
 }
}
pub(crate) fn may_show(monitor_is_primary: bool) -> bool { !monitor_is_primary }
#[cfg(test)]
fn offscreen_origin_outside_displays(origin: Point<Pixels>, displays: &[DisplayInfo]) -> bool {
 displays.iter().all(|display| {
  let bounds = display.bounds;
  origin.x < bounds.origin.x || origin.y < bounds.origin.y ||
   origin.x >= bounds.origin.x + bounds.size.width || origin.y >= bounds.origin.y + bounds.size.height
 })
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

// gpui does not re-export raw-window-handle, so identify the newly created
// top-level HWND by comparing this process's windows around open_window.
#[cfg(windows)]
pub(crate) fn process_windows() -> Result<Vec<isize>, String> {
 use windows_sys::Win32::{Foundation::{BOOL, HWND, LPARAM}, UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId}};
 unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> BOOL {
  let mut pid = 0;
  unsafe { GetWindowThreadProcessId(hwnd, &mut pid); }
  if pid == std::process::id() { unsafe { (*(param as *mut Vec<isize>)).push(hwnd as isize); } }
  1
 }
 let mut windows = Vec::new();
 if unsafe { EnumWindows(Some(collect), &mut windows as *mut _ as LPARAM) } == 0 {
  return Err(format!("cannot enumerate windows: {}", std::io::Error::last_os_error()));
 }
 Ok(windows)
}

#[cfg(windows)]
pub(crate) fn verify_and_show(chosen: &Chosen, selected_id: Option<DisplayId>, size: Size<Pixels>, displays: &[DisplayInfo], scale: f32, previous: &[isize]) -> Result<(), String> {
 use windows_sys::Win32::{Foundation::{HWND, POINT}, Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MonitorFromPoint, MonitorFromWindow, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY}, UI::WindowsAndMessaging::{SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE}};
 let new: Vec<_> = process_windows()?.into_iter().filter(|hwnd| !previous.contains(hwnd)).collect();
 let hwnd = match new.as_slice() {
  [hwnd] => *hwnd as HWND,
  _ => return Err(format!("cannot identify main window (found {} new process windows)", new.len())),
 };
 // Hidden gpui windows retain their requested placement internally and are
 // initially created with CW_USEDEFAULT. Apply it before verification.
 let mut expected_monitor = None;
 let (x, y, width, height, flags) = match chosen {
  Chosen::Display { id, origin } => {
   // On gpui's Windows backend the full DisplayId is the HMONITOR. Keep
   // the full value obtained from cx.displays(), not the truncated u32 key.
   let monitor = u64::from(selected_id.ok_or_else(|| format!("display {id} has no corresponding monitor"))?) as usize as HMONITOR;
   let display = displays.iter().find(|display| display.id == *id).ok_or_else(|| format!("display {id} is no longer available"))?;
   let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
   info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
   if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 { return Err(format!("cannot query monitor {monitor:p}: {}", std::io::Error::last_os_error())); }
   let display_scale = (info.rcMonitor.right - info.rcMonitor.left) as f32 / f32::from(display.bounds.size.width);
   if !display_scale.is_finite() || display_scale <= 0. { return Err(format!("invalid scale for monitor {monitor:p}")); }
   expected_monitor = Some(monitor);
   ((f32::from(origin.x) * display_scale).round() as i32, (f32::from(origin.y) * display_scale).round() as i32,
    (f32::from(size.width) * display_scale).round() as i32, (f32::from(size.height) * display_scale).round() as i32, 0)
  },
  Chosen::Offscreen => (0, 0, (f32::from(size.width) * scale).round() as i32, (f32::from(size.height) * scale).round() as i32, SWP_NOMOVE),
 };
 if unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, flags | SWP_NOACTIVATE | SWP_NOZORDER) } == 0 {
  return Err(format!("cannot set hidden window bounds: {}", std::io::Error::last_os_error()));
 }
 if matches!(chosen, Chosen::Offscreen) && unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), -3000, -3000, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER) } == 0 {
  return Err(format!("cannot move window offscreen: {}", std::io::Error::last_os_error()));
 }
 let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
 let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
 if primary.is_null() || monitor.is_null() { return Err(format!("cannot determine window monitor (monitor {monitor:p}, primary {primary:p})")); }
 if !may_show(monitor == primary) { return Err(format!("window landed on primary monitor {monitor:p}")); }
 if expected_monitor.is_some_and(|expected| expected != monitor) { return Err(format!("window landed on monitor {monitor:p}, not the selected monitor {:p}", expected_monitor.unwrap())); }
 unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE); }
 Ok(())
}

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
 fn secondary_window_inputs_select_display_and_start_hidden() {
  let chosen = choose(Place::Secondary, &displays());
  let inputs = window_inputs(&chosen, size(px(800.), px(600.)));
  assert_eq!(inputs.display_id, Some(2));
  assert!(!inputs.show);
  assert!(!inputs.focus);
 }
 #[test]
 fn placements_show_only_off_primary() {
  let displays = displays();
  for place in [Place::Secondary, Place::Offscreen] {
   for (layout, expected_show) in [(&displays[..], true), (&displays[..1], false)] {
    let chosen = choose(place, layout);
    let monitor_is_primary = match &chosen {
     Chosen::Display { id, .. } => layout.iter().find(|display| display.id == *id).unwrap().primary,
     Chosen::Offscreen => {
      let bounds = window_bounds(&chosen, size(px(800.), px(600.)));
      let distance = |display: &&DisplayInfo| {
       let b = display.bounds;
       let dx = f32::from((b.origin.x - (bounds.origin.x + bounds.size.width)).max(bounds.origin.x - (b.origin.x + b.size.width)).max(px(0.)));
       let dy = f32::from((b.origin.y - (bounds.origin.y + bounds.size.height)).max(bounds.origin.y - (b.origin.y + b.size.height)).max(px(0.)));
       dx * dx + dy * dy
      };
      layout.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap().primary
     },
    };
    assert_eq!(may_show(monitor_is_primary), expected_show, "{place:?}");
   }
  }
 }
 #[test]
 fn offscreen_target_is_outside_single_display_and_selects_no_display() {
  let displays = displays();
  let chosen = choose(Place::Offscreen, &displays[..1]);
  assert_eq!(chosen, Chosen::Offscreen);
  let inputs = window_inputs(&chosen, size(px(800.), px(600.)));
  assert_eq!(inputs.bounds.origin, point(px(-3000.), px(-3000.)));
  assert_eq!(inputs.display_id, None);
  assert!(offscreen_origin_outside_displays(inputs.bounds.origin, &displays[..1]));
  assert!(!offscreen_origin_outside_displays(point(px(0.), px(0.)), &displays[..1]));
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
