use crate::codex::{extreme, slots};

pub(super) fn command(args: &[String]) -> i32 {
    let home = slots::home();
    let setting = match args {
        [action] if action == "status" => Ok(extreme::load(&home)),
        [action] if action == "on" || action == "off" => extreme::set(&home, action == "on"),
        _ => return super::usage("extreme: expected on, off, or status"),
    };
    match setting {
        Ok(setting) => {
            let active = setting.on
                && setting
                    .last_check
                    .as_ref()
                    .is_some_and(|c| c.status == extreme::TierStatus::Accepted);
            println!(
                "{}",
                serde_json::json!({"on": setting.on, "last_check": setting.last_check, "active": active})
            );
            0
        }
        Err(e) => {
            eprintln!("axiom: {}", e.message);
            1
        }
    }
}
